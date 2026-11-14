use sqlx::PgPool;

use super::decisions::Decision;
use super::messages::{IncomingMessage, WINDOW};
use super::provider::{JUDGED, Provider};
use super::{decisions::Decisions, messages::Messages, webhooks::Webhooks};
use crate::error::AppError;

#[derive(Clone)]
pub(crate) struct Engine {
    pub(crate) messages: Messages,
    pub(crate) decisions: Decisions,
    pub(crate) webhooks: Webhooks,
}

impl Engine {
    pub(crate) fn new(pool: PgPool, provider: Provider) -> Result<Self, reqwest::Error> {
        Ok(Self {
            messages: Messages::new(pool.clone()),
            decisions: Decisions::new(pool.clone(), provider),
            webhooks: Webhooks::new(pool)?,
        })
    }

    pub(crate) async fn submit_sync(
        &self,
        tenant: &str,
        messages: Vec<IncomingMessage>,
    ) -> Result<Vec<Decision>, AppError> {
        validate_all(&messages)?;
        self.upsert_all(tenant, &messages).await?;
        let mut decisions = Vec::with_capacity(messages.len());
        for (index, message) in messages.iter().enumerate() {
            decisions.push(
                self.decide(tenant, message, &later_ids(&messages, index))
                    .await?,
            );
        }
        Ok(decisions)
    }

    pub(crate) async fn submit_async(
        &self,
        tenant: &str,
        messages: Vec<IncomingMessage>,
    ) -> Result<(), AppError> {
        validate_all(&messages)?;
        self.upsert_all(tenant, &messages).await?;
        for message in &messages {
            self.decisions.mark_pending(tenant, message).await?;
        }
        let engine = self.clone();
        let tenant = tenant.to_owned();
        tokio::spawn(async move {
            engine.finish(&tenant, &messages).await;
        });
        Ok(())
    }

    async fn finish(&self, tenant: &str, messages: &[IncomingMessage]) {
        for (index, message) in messages.iter().enumerate() {
            if let Err(err) = self
                .deliver_decision(tenant, message, &later_ids(messages, index))
                .await
            {
                log_background(&err);
            }
        }
    }

    async fn upsert_all(&self, tenant: &str, messages: &[IncomingMessage]) -> Result<(), AppError> {
        for message in messages {
            self.messages.upsert(tenant, message).await?;
        }
        Ok(())
    }

    async fn decide(
        &self,
        tenant: &str,
        message: &IncomingMessage,
        later: &[String],
    ) -> Result<Decision, AppError> {
        let state = self.state(tenant, message, later).await?;
        self.decisions.decide(tenant, message, &state).await
    }

    async fn deliver_decision(
        &self,
        tenant: &str,
        message: &IncomingMessage,
        later: &[String],
    ) -> Result<(), AppError> {
        let decision = self.decide(tenant, message, later).await?;
        self.webhooks.deliver_decision(tenant, &decision).await
    }

    async fn state(
        &self,
        tenant: &str,
        message: &IncomingMessage,
        later: &[String],
    ) -> Result<String, AppError> {
        let mut window = self
            .messages
            .window(tenant, &message.room_id, WINDOW, later)
            .await?;
        if !window
            .iter()
            .any(|existing| existing.message_id == message.message_id)
        {
            window.push(message.clone());
            window.sort_by_key(|existing| existing.timestamp);
        }
        Ok(lines(&window, &message.message_id))
    }
}

fn validate_all(messages: &[IncomingMessage]) -> Result<(), AppError> {
    for message in messages {
        validate(message)?;
    }
    Ok(())
}

fn validate(message: &IncomingMessage) -> Result<(), AppError> {
    for (field, value) in [
        ("message_id", message.message_id.as_str()),
        ("room_id", message.room_id.as_str()),
        ("user_id", message.user_id.as_str()),
        ("text", message.text.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(AppError::MessageIncomplete { field });
        }
    }
    Ok(())
}

fn lines(messages: &[IncomingMessage], judged_id: &str) -> String {
    messages
        .iter()
        .map(|message| {
            let mark = if message.message_id == judged_id {
                JUDGED
            } else {
                ""
            };
            format!("{mark}{}: {}", message.user_id, message.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn later_ids(messages: &[IncomingMessage], index: usize) -> Vec<String> {
    messages[index + 1..]
        .iter()
        .filter(|later| later.room_id == messages[index].room_id)
        .map(|later| later.message_id.clone())
        .collect()
}

fn log_background(err: &AppError) {
    match err {
        AppError::Database { action } => tracing::error!(action, "background submit failed"),
        AppError::Provider(fault) => {
            tracing::error!(fault = ?fault, "background submit failed")
        }
        AppError::MessageIncomplete { field } => {
            tracing::error!(field, "background submit failed")
        }
        AppError::Unauthorized
        | AppError::InvalidMessage
        | AppError::WebhookUrlRequired
        | AppError::WebhookUrlInvalid
        | AppError::WebhookEncode => tracing::error!("background submit failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(message_id: &str, room_id: &str, user_id: &str, text: &str) -> IncomingMessage {
        IncomingMessage {
            message_id: message_id.to_owned(),
            room_id: room_id.to_owned(),
            user_id: user_id.to_owned(),
            text: text.to_owned(),
            timestamp: 1,
            reply_to: None,
            meta: json!({}),
        }
    }

    #[test]
    fn empty_ids_and_text_are_incomplete() {
        for (field, message) in [
            ("message_id", message("  ", "r", "u", "hi")),
            ("room_id", message("m", "", "u", "hi")),
            ("user_id", message("m", "r", " ", "hi")),
            ("text", message("m", "r", "u", "   ")),
        ] {
            assert!(matches!(
                validate(&message),
                Err(AppError::MessageIncomplete { field: got }) if got == field
            ));
        }
        assert!(validate(&message("m", "r", "u", "hi")).is_ok());
    }

    #[test]
    fn state_marks_only_the_message_being_judged() {
        let messages = vec![
            message("m1", "r", "u1", "you're an idiot"),
            message("m2", "r", "u2", "hello"),
            message("m3", "r", "u3", "ok"),
        ];
        assert_eq!(
            lines(&messages, "m2"),
            "u1: you're an idiot\n[this] u2: hello\nu3: ok"
        );
    }

    #[test]
    fn later_messages_in_the_batch_are_not_context_yet() {
        let messages = vec![
            message("m1", "r", "u1", "you're an idiot"),
            message("m2", "r", "u2", "hello"),
            message("m3", "other", "u1", "ok"),
            message("m4", "r", "u1", "idiot"),
        ];
        assert_eq!(
            later_ids(&messages, 0),
            vec!["m2".to_owned(), "m4".to_owned()]
        );
        assert!(later_ids(&messages, 3).is_empty());
    }
}
