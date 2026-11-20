use super::{IncomingMessage, Message};
use crate::error::{AppError, database};

pub(super) async fn insert(
    connection: &mut sqlx::PgConnection,
    tenant: &str,
    message: &IncomingMessage,
    retained: bool,
) -> Result<Message, AppError> {
    sqlx::query_as(
        r#"
        INSERT INTO messages (
            tenant_id, client_message_id, room_id, user_id, text, timestamp, reply_to, meta, retained
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9
        )
        RETURNING id, room_id, user_id, text
        "#,
    )
    .bind(tenant)
    .bind(&message.client_message_id)
    .bind(&message.room_id)
    .bind(&message.user_id)
    .bind(&message.text)
    .bind(message.timestamp)
    .bind(&message.reply_to)
    .bind(&message.meta)
    .bind(retained)
    .fetch_one(&mut *connection)
    .await
    .map_err(|err| database("saving a message", err))
}

pub(super) async fn room(
    pool: &sqlx::PgPool,
    tenant: &str,
    room: &str,
    limit: i64,
) -> Result<Vec<Message>, AppError> {
    sqlx::query_as(
        r#"
        SELECT id, room_id, user_id, text
        FROM (
            SELECT *
            FROM messages
            WHERE tenant_id = $1
              AND room_id = $2
              AND retained
            ORDER BY timestamp DESC
            LIMIT $3
        ) recent
        ORDER BY timestamp ASC
        "#,
    )
    .bind(tenant)
    .bind(room)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|err| database("loading a message window", err))
}

pub(super) async fn get_many(
    pool: &sqlx::PgPool,
    tenant: &str,
    ids: &[i64],
) -> Result<Vec<Message>, AppError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        r#"
        SELECT m.id, m.room_id, m.user_id, m.text
        FROM unnest($2::bigint[]) WITH ORDINALITY AS wanted(id, ord)
        JOIN messages m
          ON m.tenant_id = $1
         AND m.id = wanted.id
        ORDER BY wanted.ord
        "#,
    )
    .bind(tenant)
    .bind(ids)
    .fetch_all(pool)
    .await
    .map_err(|err| database("loading messages", err))
}

pub(super) async fn is_retained(pool: &sqlx::PgPool, id: i64) -> Result<bool, AppError> {
    let retained = sqlx::query_scalar(
        r#"
        SELECT retained
        FROM messages
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| database("reading a message", err))?;
    Ok(retained.unwrap_or(false))
}
