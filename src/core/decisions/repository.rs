use sqlx::{PgPool, types::Json};

use super::{Action, Category, CategoryScores, DecidedBy, Decision, Status};
use crate::core::messages::IncomingMessage;
use crate::error::{AppError, database};

pub(super) async fn mark_pending(
    pool: &PgPool,
    tenant: &str,
    message: &IncomingMessage,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO decisions (
            tenant_id, message_id, room_id, status,
            action, categories, triggered, decided_by, latency_ms
        ) VALUES (
            $1, $2, $3, $4,
            NULL, NULL, NULL, NULL, NULL
        )
        ON CONFLICT (tenant_id, message_id) DO UPDATE
        SET room_id = EXCLUDED.room_id,
            status = EXCLUDED.status,
            action = NULL,
            categories = NULL,
            triggered = NULL,
            decided_by = NULL,
            latency_ms = NULL,
            updated_at = now()
        "#,
    )
    .bind(tenant)
    .bind(&message.message_id)
    .bind(&message.room_id)
    .bind(Status::Pending)
    .execute(pool)
    .await
    .map_err(|err| database("marking a decision pending", err))?;
    Ok(())
}

pub(super) async fn write_ready(
    pool: &PgPool,
    tenant: &str,
    message: &IncomingMessage,
    chosen: Action,
    categories: &CategoryScores,
    hits: &[Category],
    elapsed: i64,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO decisions (
            tenant_id, message_id, room_id, status,
            action, categories, triggered, decided_by, latency_ms
        ) VALUES (
            $1, $2, $3, $4,
            $5, $6, $7, $8, $9
        )
        ON CONFLICT (tenant_id, message_id) DO UPDATE
        SET room_id = EXCLUDED.room_id,
            status = EXCLUDED.status,
            action = EXCLUDED.action,
            categories = EXCLUDED.categories,
            triggered = EXCLUDED.triggered,
            decided_by = EXCLUDED.decided_by,
            latency_ms = EXCLUDED.latency_ms,
            updated_at = now()
        "#,
    )
    .bind(tenant)
    .bind(&message.message_id)
    .bind(&message.room_id)
    .bind(Status::Ready)
    .bind(chosen)
    .bind(Json(categories))
    .bind(Json(hits))
    .bind(DecidedBy::Model)
    .bind(elapsed)
    .execute(pool)
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
) -> Result<Vec<Decision>, AppError> {
    sqlx::query_as(
        r#"
        SELECT d.message_id,
               d.status,
               d.action,
               d.categories,
               COALESCE(d.triggered, '[]'::jsonb) AS triggered,
               d.decided_by,
               d.latency_ms
        FROM decisions d
        JOIN messages m
          ON m.tenant_id = d.tenant_id
         AND m.message_id = d.message_id
        WHERE d.tenant_id = $1
          AND d.room_id = $2
          AND (cardinality($3::text[]) = 0 OR d.message_id = ANY($3::text[]))
          AND ($4::text IS NULL OR m.user_id = $4)
          AND ($5::bigint IS NULL OR m.timestamp > $5)
        ORDER BY m.timestamp ASC, d.message_id ASC
        "#,
    )
    .bind(tenant)
    .bind(room)
    .bind(ids)
    .bind(user_id)
    .bind(after)
    .fetch_all(pool)
    .await
    .map_err(|err| database("listing decisions", err))
}
