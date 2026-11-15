use std::collections::BTreeMap;

use sqlx::PgPool;

use super::decisions::Decision;
use super::messages::IncomingMessage;
use super::provider::{JUDGED, Provider};
use super::{decisions::Decisions, messages::Messages, webhooks::Webhooks};
use crate::error::AppError;

type RoomWindows = BTreeMap<String, Vec<IncomingMessage>>;

#[derive(Clone)]
pub(crate) struct Engine {
    pub(crate) messages: Messages,
    pub(crate) decisions: Decisions,
    pub(crate) webhooks: Webhooks,
}

impl Engine {
    pub(crate) fn new(
        pool: PgPool,
        provider: Provider,
        window: i64,
        threshold: f64,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            messages: Messages::new(pool.clone(), window),
            decisions: Decisions::new(pool.clone(), provider, threshold),
            webhooks: Webhooks::new(pool)?,
        })
    }

    pub(crate) async fn submit_sync(
        &self,
        tenant: &str,
        messages: Vec<IncomingMessage>,
    ) -> Result<Vec<Decision>, AppError> {
        validate_all(&messages)?;
        let windows = self.windows(tenant, &messages).await?;
        self.upsert_all(tenant, &messages).await?;
        let ids = self.decisions.open(tenant, &messages).await?;
        let mut decisions = Vec::with_capacity(messages.len());
        let mut first_error = None;
        for (index, (message, id)) in messages.iter().zip(ids).enumerate() {
            let state = state(&windows, &messages, index);
            match self.decisions.score(id, message, &state).await {
                Ok(decision) => decisions.push(decision),
                Err(err) => {
                    self.decisions.mark_failed(id).await?;
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                }
            }
        }
        if let Some(err) = first_error {
            return Err(err);
        }
        Ok(decisions)
    }

    pub(crate) async fn submit_async(
        &self,
        tenant: &str,
        messages: Vec<IncomingMessage>,
    ) -> Result<(), AppError> {
        validate_all(&messages)?;
        let windows = self.windows(tenant, &messages).await?;
        self.upsert_all(tenant, &messages).await?;
        let ids = self.decisions.open(tenant, &messages).await?;
        if ids.is_empty() {
            return Ok(());
        }
        let jobs: Vec<(usize, i64)> = ids.into_iter().enumerate().collect();
        let engine = self.clone();
        let tenant = tenant.to_owned();
        tokio::spawn(async move {
            engine.finish(&tenant, &messages, &windows, &jobs).await;
        });
        Ok(())
    }

    async fn finish(
        &self,
        tenant: &str,
        messages: &[IncomingMessage],
        windows: &RoomWindows,
        jobs: &[(usize, i64)],
    ) {
        for (index, id) in jobs {
            let state = state(windows, messages, *index);
            match self.decisions.score(*id, &messages[*index], &state).await {
                Ok(decision) => {
                    if let Err(err) = self.webhooks.deliver_decision(tenant, &decision).await {
                        log_background(&err);
                    }
                }
                Err(err) => {
                    if let Err(mark_err) = self.decisions.mark_failed(*id).await {
                        log_background(&mark_err);
                    }
                    log_background(&err);
                }
            }
        }
    }

    async fn windows(
        &self,
        tenant: &str,
        messages: &[IncomingMessage],
    ) -> Result<RoomWindows, AppError> {
        let mut rooms = Vec::new();
        for message in messages {
            if !rooms.iter().any(|room: &String| room == &message.room_id) {
                rooms.push(message.room_id.clone());
            }
        }
        let mut windows = RoomWindows::new();
        for room in rooms {
            let window = self.messages.window(tenant, &room).await?;
            windows.insert(room, window);
        }
        Ok(windows)
    }

    async fn upsert_all(&self, tenant: &str, messages: &[IncomingMessage]) -> Result<(), AppError> {
        for message in messages {
            self.messages.upsert(tenant, message).await?;
        }
        Ok(())
    }
}

fn state(windows: &RoomWindows, messages: &[IncomingMessage], index: usize) -> String {
    let window = context(
        windows
            .get(&messages[index].room_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        messages,
        index,
    );
    lines(&window, &messages[index].message_id)
}

fn context(
    prior: &[IncomingMessage],
    batch: &[IncomingMessage],
    index: usize,
) -> Vec<IncomingMessage> {
    let room = &batch[index].room_id;
    let mut window = prior.to_vec();
    for message in batch[..=index]
        .iter()
        .filter(|message| &message.room_id == room)
    {
        if let Some(existing) = window
            .iter_mut()
            .find(|row| row.message_id == message.message_id)
        {
            *existing = message.clone();
        } else {
            window.push(message.clone());
        }
    }
    window
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
        | AppError::UnknownCategory
        | AppError::WebhookUrlRequired
        | AppError::WebhookUrlInvalid
        | AppError::WebhookEncode
        | AppError::InvalidLimit => tracing::error!("background submit failed"),
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
    fn the_batch_is_applied_onto_the_room_in_order() {
        let prior = vec![at(message("m0", "r", "u0", "earlier"), 1)];
        let batch = vec![
            at(message("m1", "r", "u1", "you're an idiot"), 3),
            at(message("m2", "r", "u2", "hello"), 4),
            at(message("m3", "other", "u1", "ok"), 5),
            at(message("m4", "r", "u1", "idiot"), 6),
        ];
        assert_eq!(
            lines(&context(&prior, &batch, 0), "m1"),
            "u0: earlier\n[this] u1: you're an idiot"
        );
        assert_eq!(
            lines(&context(&prior, &batch, 1), "m2"),
            "u0: earlier\nu1: you're an idiot\n[this] u2: hello"
        );
        assert_eq!(lines(&context(&[], &batch, 2), "m3"), "[this] u1: ok");
        let fourth = context(&prior, &batch, 3);
        assert!(fourth.iter().any(|message| message.message_id == "m1"));
        assert!(fourth.iter().any(|message| message.message_id == "m2"));
        assert!(fourth.iter().all(|message| message.room_id == "r"));
    }

    #[test]
    fn an_incoming_line_replaces_the_stored_copy() {
        let prior = vec![at(message("m1", "r", "u1", "old"), 1)];
        let batch = vec![
            at(message("m1", "r", "u1", "first"), 2),
            at(message("m1", "r", "u1", "second"), 3),
        ];
        let window = context(&prior, &batch, 1);
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].text, "second");
    }

    #[test]
    fn the_batch_is_appended_in_request_order() {
        let prior = vec![at(message("m0", "r", "u0", "stored"), 5)];
        let batch = vec![
            at(message("m1", "r", "u1", "first"), 1),
            at(message("m2", "r", "u2", "second"), 9),
        ];
        assert_eq!(
            lines(&context(&prior, &batch, 1), "m2"),
            "u0: stored\nu1: first\n[this] u2: second"
        );
    }

    fn at(mut message: IncomingMessage, timestamp: i64) -> IncomingMessage {
        message.timestamp = timestamp;
        message
    }
}
