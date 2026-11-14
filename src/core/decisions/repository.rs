use sqlx::{PgPool, Postgres, Transaction};

use super::{CategoryScores, Decision, Status};
use crate::core::category::Action;
use crate::core::messages::IncomingMessage;
use crate::error::{AppError, database};

pub(super) async fn mark_pending(
    pool: &PgPool,
    tenant: &str,
    message: &IncomingMessage,
) -> Result<(), AppError> {
    mark(
        pool,
        tenant,
        message,
        Status::Pending,
        "marking a decision pending",
    )
    .await
}

pub(super) async fn mark_failed(
    pool: &PgPool,
    tenant: &str,
    message: &IncomingMessage,
) -> Result<(), AppError> {
    mark(
        pool,
        tenant,
        message,
        Status::Failed,
        "marking a decision failed",
    )
    .await
}

async fn mark(
    pool: &PgPool,
    tenant: &str,
    message: &IncomingMessage,
    status: Status,
    action: &'static str,
) -> Result<(), AppError> {
    let mut tx = pool.begin().await.map_err(|err| database(action, err))?;
    sqlx::query(
        r#"
        INSERT INTO decisions (tenant_id, message_id, status, action)
        VALUES ($1, $2, $3, NULL)
        ON CONFLICT (tenant_id, message_id) DO UPDATE
        SET status = EXCLUDED.status,
            action = NULL,
            updated_at = now()
        "#,
    )
    .bind(tenant)
    .bind(&message.message_id)
    .bind(status)
    .execute(&mut *tx)
    .await
    .map_err(|err| database(action, err))?;
    delete_scores(&mut tx, tenant, &message.message_id).await?;
    tx.commit().await.map_err(|err| database(action, err))?;
    Ok(())
}

pub(super) async fn write_ready(
    pool: &PgPool,
    tenant: &str,
    message: &IncomingMessage,
    chosen: Action,
    categories: &CategoryScores,
) -> Result<(), AppError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| database("saving a decision", err))?;
    sqlx::query(
        r#"
        INSERT INTO decisions (tenant_id, message_id, status, action)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (tenant_id, message_id) DO UPDATE
        SET status = EXCLUDED.status,
            action = EXCLUDED.action,
            updated_at = now()
        "#,
    )
    .bind(tenant)
    .bind(&message.message_id)
    .bind(Status::Ready)
    .bind(chosen)
    .execute(&mut *tx)
    .await
    .map_err(|err| database("saving a decision", err))?;
    delete_scores(&mut tx, tenant, &message.message_id).await?;
    insert_scores(&mut tx, tenant, &message.message_id, categories).await?;
    tx.commit()
        .await
        .map_err(|err| database("saving a decision", err))?;
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
) -> Result<Vec<Decision>, AppError> {
    sqlx::query_as(
        r#"
        SELECT d.message_id,
               d.status,
               d.action,
               scores.categories
        FROM decisions d
        JOIN messages m
          ON m.tenant_id = d.tenant_id
         AND m.message_id = d.message_id
        LEFT JOIN LATERAL (
            SELECT jsonb_object_agg(s.category, s.score) AS categories
            FROM decision_scores s
            WHERE s.tenant_id = d.tenant_id
              AND s.message_id = d.message_id
        ) scores ON true
        WHERE d.tenant_id = $1
          AND m.room_id = $2
          AND (cardinality($3::text[]) = 0 OR d.message_id = ANY($3::text[]))
          AND ($4::text IS NULL OR m.user_id = $4)
          AND ($5::bigint IS NULL OR m.timestamp > $5)
          AND (
            $6::text IS NULL
            OR EXISTS (
                SELECT 1
                FROM decision_scores hit
                WHERE hit.tenant_id = d.tenant_id
                  AND hit.message_id = d.message_id
                  AND hit.category = $6
                  AND hit.score >= $7
            )
          )
        ORDER BY m.timestamp ASC, d.message_id ASC
        "#,
    )
    .bind(tenant)
    .bind(room)
    .bind(ids)
    .bind(user_id)
    .bind(after)
    .bind(category)
    .bind(threshold)
    .fetch_all(pool)
    .await
    .map_err(|err| database("listing decisions", err))
}

async fn delete_scores(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &str,
    message_id: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        DELETE FROM decision_scores
        WHERE tenant_id = $1
          AND message_id = $2
        "#,
    )
    .bind(tenant)
    .bind(message_id)
    .execute(&mut **tx)
    .await
    .map_err(|err| database("replacing decision scores", err))?;
    Ok(())
}

async fn insert_scores(
    tx: &mut Transaction<'_, Postgres>,
    tenant: &str,
    message_id: &str,
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
        INSERT INTO decision_scores (tenant_id, message_id, category, score)
        SELECT $1, $2, category, score
        FROM unnest($3::text[], $4::float8[]) AS score_row(category, score)
        "#,
    )
    .bind(tenant)
    .bind(message_id)
    .bind(&names)
    .bind(&scores)
    .execute(&mut **tx)
    .await
    .map_err(|err| database("saving a decision", err))?;
    Ok(())
}
