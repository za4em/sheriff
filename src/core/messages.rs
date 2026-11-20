mod repository;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::error::AppError;

/// A message from a client. The row id is assigned by the database.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub(crate) struct IncomingMessage {
    #[serde(rename = "message_id")]
    pub(crate) client_message_id: String,
    pub(crate) room_id: String,
    pub(crate) user_id: String,
    pub(crate) text: String,
    pub(crate) timestamp: i64,
    #[serde(default)]
    pub(crate) reply_to: Option<String>,
    #[serde(default = "empty_object")]
    pub(crate) meta: Value,
}

/// A message row. `row_id` is `messages.id`.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct Message {
    #[sqlx(rename = "id")]
    pub(crate) row_id: i64,
    pub(crate) room_id: String,
    pub(crate) user_id: String,
    pub(crate) text: String,
}

#[derive(Clone)]
pub(crate) struct Messages {
    pool: PgPool,
    window: i64,
}

/// Inserts on the caller's transaction. The engine owns that transaction.
pub(crate) async fn insert_on(
    connection: &mut sqlx::PgConnection,
    tenant: &str,
    message: &IncomingMessage,
    retained: bool,
) -> Result<Message, AppError> {
    repository::insert(connection, tenant, message, retained).await
}

impl Messages {
    pub(crate) fn new(pool: PgPool, window: i64) -> Self {
        Self { pool, window }
    }

    pub(crate) async fn room(&self, tenant: &str, room: &str) -> Result<Vec<Message>, AppError> {
        repository::room(&self.pool, tenant, room, self.window).await
    }

    pub(crate) async fn get_many(
        &self,
        tenant: &str,
        ids: &[i64],
    ) -> Result<Vec<Message>, AppError> {
        repository::get_many(&self.pool, tenant, ids).await
    }

    pub(crate) async fn is_retained(&self, id: i64) -> Result<bool, AppError> {
        repository::is_retained(&self.pool, id).await
    }
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_reply_and_meta_default() {
        let messages: Vec<IncomingMessage> = serde_json::from_str(
            r#"[{"message_id":"m1","room_id":"r","user_id":"u","text":"hi","timestamp":1}]"#,
        )
        .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].client_message_id, "m1");
        assert_eq!(messages[0].reply_to, None);
        assert!(messages[0].meta.as_object().unwrap().is_empty());
    }

    #[test]
    fn a_client_supplied_row_id_is_not_part_of_the_message() {
        let message: IncomingMessage = serde_json::from_str(
            r#"{"id":42,"row_id":42,"message_id":"m1","room_id":"r","user_id":"u","text":"hi","timestamp":1}"#,
        )
        .unwrap();
        let stored = serde_json::to_value(&message).unwrap();
        assert!(stored.get("id").is_none());
        assert!(stored.get("row_id").is_none());
        assert_eq!(stored["message_id"], "m1");
    }
}
