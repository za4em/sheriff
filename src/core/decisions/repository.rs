use sqlx::PgPool;

use super::{CategoryScores, Decision, Status};
use crate::core::category::Action;
use crate::core::messages::IncomingMessage;
use crate::error::{AppError, database};

pub(super) async fn open(
    pool: &PgPool,
    tenant: &str,
    messages: &[IncomingMessage],
) -> Result<Vec<i64>, AppError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| database("opening a decision (begin)", err))?;
    let mut ids = Vec::with_capacity(messages.len());
    for message in messages {
        let id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO decisions (tenant_id, message_id, status)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
        )
        .bind(tenant)
        .bind(&message.message_id)
        .bind(Status::Pending)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| database("opening a decision (insert)", err))?;
        ids.push(id);
    }
    tx.commit()
        .await
        .map_err(|err| database("opening a decision (commit)", err))?;
    Ok(ids)
}

pub(super) async fn write_ready(
    pool: &PgPool,
    decision_id: i64,
    chosen: Action,
    categories: &CategoryScores,
) -> Result<(), AppError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| database("saving a decision (begin)", err))?;
    let saved = sqlx::query(
        r#"
        UPDATE decisions
        SET status = $2,
            action = $3,
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(decision_id)
    .bind(Status::Ready)
    .bind(chosen)
    .execute(&mut *tx)
    .await
    .map_err(|err| database("saving a decision (update)", err))?;
    if saved.rows_affected() == 0 {
        tracing::error!("decision row was missing while saving a decision");
        return Err(AppError::Database {
            action: "saving a decision (missing)",
        });
    }
    insert_scores(&mut *tx, decision_id, categories).await?;
    tx.commit()
        .await
        .map_err(|err| database("saving a decision (commit)", err))?;
    Ok(())
}

pub(super) async fn mark_failed(pool: &PgPool, decision_id: i64) -> Result<(), AppError> {
    let saved = sqlx::query(
        r#"
        UPDATE decisions
        SET status = $2,
            action = NULL,
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(decision_id)
    .bind(Status::Failed)
    .execute(pool)
    .await
    .map_err(|err| database("marking a decision failed (update)", err))?;
    if saved.rows_affected() == 0 {
        tracing::error!("decision row was missing while marking a decision failed");
        return Err(AppError::Database {
            action: "marking a decision failed (missing)",
        });
    }
    Ok(())
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
        SELECT d.message_id,
               d.status,
               d.action,
               scores.categories
        FROM messages m
        JOIN LATERAL (
            SELECT id, message_id, status, action
            FROM decisions
            WHERE tenant_id = m.tenant_id
              AND message_id = m.message_id
            ORDER BY id DESC
            LIMIT 1
        ) d ON true
        LEFT JOIN LATERAL (
            SELECT jsonb_object_agg(s.category, s.score) AS categories
            FROM decision_scores s
            WHERE s.decision_id = d.id
              AND s.category = ANY($8::text[])
        ) scores ON true
        WHERE m.tenant_id = $1
          AND m.room_id = $2
          AND (cardinality($3::text[]) = 0 OR m.message_id = ANY($3::text[]))
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
        ORDER BY m.timestamp ASC, m.message_id ASC
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
