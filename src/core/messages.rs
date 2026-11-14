mod repository;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;

use crate::error::AppError;

pub(crate) const WINDOW: i64 = 50;

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, sqlx::FromRow)]
pub(crate) struct IncomingMessage {
    pub(crate) message_id: String,
    pub(crate) room_id: String,
    pub(crate) user_id: String,
    pub(crate) text: String,
    pub(crate) timestamp: i64,
    #[serde(default)]
    pub(crate) reply_to: Option<String>,
    #[serde(default = "empty_object")]
    pub(crate) meta: Value,
}

#[derive(Clone)]
pub(crate) struct Messages {
    pool: PgPool,
}

impl Messages {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) async fn upsert(
        &self,
        tenant: &str,
        message: &IncomingMessage,
    ) -> Result<(), AppError> {
        repository::upsert(&self.pool, tenant, message).await
    }

    pub(crate) async fn window(
        &self,
        tenant: &str,
        room: &str,
    ) -> Result<Vec<IncomingMessage>, AppError> {
        let mut messages = repository::window(&self.pool, tenant, room, WINDOW).await?;
        messages.reverse();
        Ok(messages)
    }
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
        assert_eq!(messages[0].message_id, "m1");
        assert_eq!(messages[0].reply_to, None);
        assert!(messages[0].meta.as_object().unwrap().is_empty());
    }
}
