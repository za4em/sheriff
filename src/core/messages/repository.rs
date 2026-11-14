use sqlx::PgPool;

use super::IncomingMessage;
use crate::error::{AppError, database};

pub(super) async fn upsert(
    pool: &PgPool,
    tenant: &str,
    message: &IncomingMessage,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO messages (
            tenant_id, message_id, room_id, user_id, text, timestamp, reply_to, meta
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8
        )
        ON CONFLICT (tenant_id, message_id) DO UPDATE
        SET text = EXCLUDED.text,
            room_id = EXCLUDED.room_id,
            user_id = EXCLUDED.user_id,
            timestamp = EXCLUDED.timestamp,
            reply_to = EXCLUDED.reply_to,
            meta = EXCLUDED.meta
        "#,
    )
    .bind(tenant)
    .bind(&message.message_id)
    .bind(&message.room_id)
    .bind(&message.user_id)
    .bind(&message.text)
    .bind(message.timestamp)
    .bind(&message.reply_to)
    .bind(&message.meta)
    .execute(pool)
    .await
    .map_err(|err| database("saving a message", err))?;
    Ok(())
}

pub(super) async fn window(
    pool: &PgPool,
    tenant: &str,
    room: &str,
    limit: i64,
) -> Result<Vec<IncomingMessage>, AppError> {
    sqlx::query_as(
        r#"
        SELECT message_id, room_id, user_id, text, timestamp, reply_to, meta
        FROM messages
        WHERE tenant_id = $1
          AND room_id = $2
        ORDER BY timestamp DESC
        LIMIT $3
        "#,
    )
    .bind(tenant)
    .bind(room)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| database("loading a message window", err))
}
