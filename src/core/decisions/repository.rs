use sqlx::PgPool;

use super::{CategoryScores, Decision, MessageWithContext, Status};
use crate::core::category::Action;
use crate::error::{AppError, database};

pub(crate) async fn insert_pending(
    connection: &mut sqlx::PgConnection,
    tenant: &str,
    messages: &[MessageWithContext],
) -> Result<Vec<i64>, AppError> {
    let mut ids = Vec::with_capacity(messages.len());
    for message in messages {
        let id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO decisions (tenant_id, message_id, status, context)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(tenant)
        .bind(message.message_row_id)
        .bind(Status::Pending)
        .bind(&message.context)
        .fetch_one(&mut *connection)
        .await
        .map_err(|err| database("opening a decision", err))?;
        ids.push(id);
    }
    Ok(ids)
}

pub(super) async fn mark_ready(
    pool: &PgPool,
    decision_id: i64,
    chosen: Action,
    categories: &CategoryScores,
) -> Result<bool, AppError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| database("saving a decision (begin)", err))?;
    let saved = sqlx::query(
        r#"
        UPDATE decisions
        SET status = $2,
            action = $3,
            error = NULL,
            updated_at = now()
        WHERE id = $1
          AND status = $4
        "#,
    )
    .bind(decision_id)
    .bind(Status::Ready)
    .bind(chosen)
    .bind(Status::Pending)
    .execute(&mut *tx)
    .await
    .map_err(|err| database("saving a decision (update)", err))?;
    if saved.rows_affected() == 0 {
        return Ok(false);
    }
    insert_scores(&mut tx, decision_id, categories).await?;
    tx.commit()
        .await
        .map_err(|err| database("saving a decision (commit)", err))?;
    Ok(true)
}

pub(super) async fn mark_failed(
    pool: &PgPool,
    decision_id: i64,
    error: &str,
) -> Result<bool, AppError> {
    let saved = sqlx::query(
        r#"
        UPDATE decisions
        SET status = $2,
            action = NULL,
            error = $3,
            updated_at = now()
        WHERE id = $1
          AND status = $4
        "#,
    )
    .bind(decision_id)
    .bind(Status::Failed)
    .bind(error)
    .bind(Status::Pending)
    .execute(pool)
    .await
    .map_err(|err| database("marking a decision failed", err))?;
    Ok(saved.rows_affected() > 0)
}

pub(super) async fn list(
    pool: &PgPool,
    tenant: &str,
    room: &str,
    ids: &[String],
    user_id: Option<&str>,
    after: Option<i64>,
    category: Option<String>,
    threshold: f64,
    categories: &[String],
    limit: i64,
) -> Result<Vec<Decision>, AppError> {
    sqlx::query_as(
        r#"
        SELECT d.id,
               m.tenant_id,
               d.message_id,
               m.client_message_id,
               d.status,
               d.action,
               scores.categories,
               d.error,
               d.context
        FROM messages m
        JOIN decisions d ON d.message_id = m.id
        LEFT JOIN LATERAL (
            SELECT jsonb_object_agg(s.category, s.score) AS categories
            FROM decision_scores s
            WHERE s.decision_id = d.id
              AND s.category = ANY($8::text[])
        ) scores ON true
        WHERE m.tenant_id = $1
          AND m.room_id = $2
          AND m.retained
          AND (cardinality($3::text[]) = 0 OR m.client_message_id = ANY($3::text[]))
          AND ($4::text IS NULL OR m.user_id = $4)
          AND ($5::bigint IS NULL OR m.timestamp > $5)
          AND (
            $6::text IS NULL
            OR EXISTS (
                SELECT 1
                FROM decision_scores hit
                WHERE hit.decision_id = d.id
                  AND hit.category = $6
                  AND hit.score >= $7
            )
          )
        ORDER BY m.timestamp ASC, m.client_message_id ASC
        LIMIT $9
        "#,
    )
    .bind(tenant)
    .bind(room)
    .bind(ids)
    .bind(user_id)
    .bind(after)
    .bind(category)
    .bind(threshold)
    .bind(categories)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| database("listing decisions", err))
}

pub(super) async fn get(pool: &PgPool, ids: &[i64]) -> Result<Vec<Decision>, AppError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        r#"
        SELECT d.id,
               d.tenant_id,
               d.message_id,
               m.client_message_id,
               d.status,
               d.action,
               scores.categories,
               d.error,
               d.context
        FROM unnest($1::bigint[]) WITH ORDINALITY AS wanted(id, ord)
        JOIN decisions d ON d.id = wanted.id
        JOIN messages m ON m.id = d.message_id
        LEFT JOIN LATERAL (
            SELECT jsonb_object_agg(category, score) AS categories
            FROM decision_scores
            WHERE decision_id = d.id
        ) scores ON true
        ORDER BY wanted.ord
        "#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .map_err(|err| database("reading decisions", err))
}

pub(super) async fn stale_pending(
    pool: &PgPool,
    seconds: i64,
    limit: i64,
) -> Result<Vec<i64>, AppError> {
    sqlx::query_scalar(
        r#"
        SELECT id
        FROM decisions
        WHERE status = $1
          AND updated_at < now() - make_interval(secs => $2)
        ORDER BY updated_at ASC
        LIMIT $3
        "#,
    )
    .bind(Status::Pending)
    .bind(seconds as f64)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| database("finding stale decisions", err))
}

pub(super) async fn delete(pool: &PgPool, ids: &[i64]) -> Result<(), AppError> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"
        WITH doomed AS (
            SELECT id, message_id, context
            FROM decisions
            WHERE id = ANY($1)
        ),
        refs AS (
            SELECT message_id AS id FROM doomed
            UNION
            SELECT unnest(context) FROM doomed
        ),
        drop_decisions AS (
            DELETE FROM decisions
            WHERE id IN (SELECT id FROM doomed)
        )
        DELETE FROM messages m
        USING refs
        WHERE m.id = refs.id
          AND NOT m.retained
          AND NOT EXISTS (
              SELECT 1
              FROM decisions d
              WHERE d.id <> ALL($1)
                AND (d.message_id = m.id OR m.id = ANY(d.context))
          )
        "#,
    )
    .bind(ids)
    .execute(pool)
    .await
    .map_err(|err| database("deleting decisions", err))?;
    Ok(())
}

async fn insert_scores(
    tx: &mut sqlx::PgConnection,
    decision_id: i64,
    categories: &CategoryScores,
) -> Result<(), AppError> {
    let mut names = Vec::with_capacity(categories.len());
    let mut scores = Vec::with_capacity(categories.len());
    for (category, score) in categories {
        names.push(category.key());
        scores.push(*score);
    }
    sqlx::query(
        r#"
        INSERT INTO decision_scores (decision_id, category, score)
        SELECT $1, category, score
        FROM unnest($2::text[], $3::float8[]) AS score_row(category, score)
        "#,
    )
    .bind(decision_id)
    .bind(&names)
    .bind(&scores)
    .execute(tx)
    .await
    .map_err(|err| database("saving decision scores", err))?;
    Ok(())
}
