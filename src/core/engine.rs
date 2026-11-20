use std::collections::BTreeMap;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::mpsc;

use super::cache::{Cache, ReadyDecision};
use super::decisions::{Decision, Decisions, MessageWithContext};
use super::messages::{IncomingMessage, Message, Messages};
use super::provider::Provider;
use super::queue::{Queue, ScoreJob};
use super::webhooks::Webhooks;
use crate::error::{AppError, database, queue};

mod sweep;
mod wait;
mod worker;

use wait::{ScoreWait, WaitList, Waiter};

pub(crate) type SocketSender = mpsc::UnboundedSender<Decision>;

const SCORE_WAIT: Duration = Duration::from_secs(5);

type RoomWindows = BTreeMap<String, Vec<Message>>;

#[derive(Clone)]
pub(crate) struct Engine {
    pool: PgPool,
    pub(crate) messages: Messages,
    pub(crate) decisions: Decisions,
    pub(crate) webhooks: Webhooks,
    queue: Queue,
    cache: Cache,
    waiting: WaitList,
}

impl Engine {
    pub(crate) fn new(
        pool: PgPool,
        provider: Provider,
        window: i64,
        threshold: f64,
        queue: Queue,
        cache: Cache,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            pool: pool.clone(),
            messages: Messages::new(pool.clone(), window),
            decisions: Decisions::new(pool.clone(), provider, threshold),
            webhooks: Webhooks::new(pool)?,
            queue,
            cache,
            waiting: WaitList::default(),
        })
    }

    /// Workers score jobs. The observer delivers sockets and webhooks.
    /// The sweeper republishes pending decisions left behind by a crash.
    pub(crate) async fn serve(&self, workers: usize) {
        let ready_decisions = self.cache.listen_decisions().await;
        let observer = self.clone();
        tokio::spawn(async move { observer.observe(ready_decisions).await });
        let jobs = self.queue.clone().listen(workers);
        let sweep_decisions = self.decisions.clone();
        let sweep_queue = self.queue.clone();
        tokio::spawn(async move { sweep::run(sweep_decisions, sweep_queue).await });
        for _ in 0..workers {
            let jobs = jobs.clone();
            let decisions = self.decisions.clone();
            let messages = self.messages.clone();
            let cache = self.cache.clone();
            tokio::spawn(async move { worker::work(jobs, decisions, messages, cache).await });
        }
    }

    async fn observe(&self, mut incoming: mpsc::UnboundedReceiver<ReadyDecision>) {
        while let Some(finished) = incoming.recv().await {
            let waiter = self.waiting.take(finished.decision_id);
            let decision = match self.decisions.get(finished.decision_id).await {
                Ok(decision) => decision,
                Err(err) => {
                    tracing::error!(error = %err, "background work failed");
                    if let Some(Waiter::Reply(done)) = waiter {
                        let _ = done.send(());
                    }
                    continue;
                }
            };
            match waiter {
                Some(Waiter::Reply(done)) => {
                    let _ = done.send(());
                }
                Some(Waiter::Socket(socket)) => {
                    if let Some(decision) = decision.clone() {
                        let _ = socket.send(decision);
                    }
                }
                None => {}
            }
            if finished.completed_now
                && let Some(decision) = decision
                && self.claim_webhook(finished.decision_id).await
                && let Err(err) = self.webhook(&decision).await
            {
                tracing::error!(error = %err, "background work failed");
            }
        }
    }

    async fn claim_webhook(&self, decision_id: i64) -> bool {
        match self.cache.claim_webhook(decision_id).await {
            Ok(claimed) => claimed,
            Err(err) => {
                tracing::error!(error = %err, "background work failed");
                false
            }
        }
    }

    async fn webhook(&self, decision: &Decision) -> Result<(), AppError> {
        if self.messages.is_retained(decision.message_row_id).await? {
            self.webhooks
                .deliver_decision(&decision.tenant, decision)
                .await?;
        }
        Ok(())
    }

    /// Stores the batch as not retained, waits until the workers finish, then deletes the rows.
    pub(crate) async fn score(
        &self,
        tenant: &str,
        messages: Vec<IncomingMessage>,
    ) -> Result<Vec<Decision>, AppError> {
        let ids = self.create_decisions(tenant, &messages, false).await?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut waits = self.watch(&ids);
        let published = self.enqueue(&ids).await;
        waits.truncate(published);
        self.wait_for(waits).await?;
        let decisions = self.decisions.get_many(&ids).await;
        if let Err(err) = self.decisions.delete(&ids).await {
            tracing::error!(error = %err, "background work failed");
        }
        let decisions = decisions?;
        if published != ids.len() {
            return Err(queue(
                "publishing a score job",
                "the broker did not accept the job",
            ));
        }
        Ok(decisions)
    }

    /// Stores the batch as retained and publishes one job per decision. Returns before the worker finishes.
    /// `reply`, when set, receives each decision as it is written.
    pub(crate) async fn submit(
        &self,
        tenant: &str,
        messages: Vec<IncomingMessage>,
        reply: Option<SocketSender>,
    ) -> Result<(), AppError> {
        let ids = self.create_decisions(tenant, &messages, true).await?;
        if ids.is_empty() {
            return Ok(());
        }
        if let Some(reply) = &reply {
            for id in &ids {
                self.waiting.socket(*id, reply.clone());
            }
        }
        self.enqueue(&ids).await;
        Ok(())
    }

    async fn create_decisions(
        &self,
        tenant: &str,
        messages: &[IncomingMessage],
        retained: bool,
    ) -> Result<Vec<i64>, AppError> {
        validate_all(messages)?;
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let windows = windows(&self.messages, tenant, messages).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| database("saving a batch", err))?;
        let mut stored = Vec::with_capacity(messages.len());
        for message in messages {
            stored.push(super::messages::insert_on(&mut tx, tenant, message, retained).await?);
        }
        let ids =
            super::decisions::insert_pending_on(&mut tx, tenant, &with_context(&windows, &stored))
                .await?;
        tx.commit()
            .await
            .map_err(|err| database("saving a batch", err))?;
        Ok(ids)
    }

    /// Publishes one job per decision. A job that does not reach the broker stays pending
    /// for the sweeper. Returns how many leading jobs were published.
    async fn enqueue(&self, ids: &[i64]) -> usize {
        for (published, id) in ids.iter().copied().enumerate() {
            if let Err(err) = self.queue.publish(&ScoreJob { decision_id: id }).await {
                tracing::error!(error = %err, decision_id = id, "score job was not published");
                return published;
            }
        }
        ids.len()
    }

    fn watch(&self, ids: &[i64]) -> Vec<ScoreWait> {
        ids.iter()
            .copied()
            .map(|id| self.waiting.reply(id))
            .collect()
    }

    async fn wait_for(&self, waits: Vec<ScoreWait>) -> Result<(), AppError> {
        let finished = async {
            for wait in waits {
                wait.finished()
                    .await
                    .map_err(|err| queue("waiting for a score", err))?;
            }
            Ok(())
        };
        match tokio::time::timeout(SCORE_WAIT, finished).await {
            Ok(result) => result,
            Err(_) => Err(AppError::ScoreTimeout),
        }
    }
}

async fn windows(
    messages: &Messages,
    tenant: &str,
    batch: &[IncomingMessage],
) -> Result<RoomWindows, AppError> {
    let mut windows = RoomWindows::new();
    for message in batch {
        if windows.contains_key(&message.room_id) {
            continue;
        }
        let window = messages.room(tenant, &message.room_id).await?;
        windows.insert(message.room_id.clone(), window);
    }
    Ok(windows)
}

fn with_context(windows: &RoomWindows, messages: &[Message]) -> Vec<MessageWithContext> {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| MessageWithContext {
            message_row_id: message.row_id,
            context: context(
                windows
                    .get(&message.room_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                messages,
                index,
            )
            .into_iter()
            .map(|line| line.row_id)
            .collect(),
        })
        .collect()
}

fn context<'a>(prior: &'a [Message], batch: &'a [Message], index: usize) -> Vec<&'a Message> {
    let room = &batch[index].room_id;
    let mut window: Vec<&Message> = prior.iter().collect();
    window.extend(
        batch[..=index]
            .iter()
            .filter(|message| &message.room_id == room),
    );
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
        ("message_id", message.client_message_id.as_str()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn incoming(message_id: &str, room_id: &str, user_id: &str, text: &str) -> IncomingMessage {
        IncomingMessage {
            client_message_id: message_id.to_owned(),
            room_id: room_id.to_owned(),
            user_id: user_id.to_owned(),
            text: text.to_owned(),
            timestamp: 1,
            reply_to: None,
            meta: json!({}),
        }
    }

    fn message(row_id: i64, room_id: &str, user_id: &str, text: &str) -> Message {
        Message {
            row_id,
            room_id: room_id.to_owned(),
            user_id: user_id.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn empty_ids_and_text_are_incomplete() {
        for (field, message) in [
            ("message_id", incoming("  ", "r", "u", "hi")),
            ("room_id", incoming("m", "", "u", "hi")),
            ("user_id", incoming("m", "r", " ", "hi")),
            ("text", incoming("m", "r", "u", "   ")),
        ] {
            assert!(matches!(
                validate(&message),
                Err(AppError::MessageIncomplete { field: got }) if got == field
            ));
        }
        assert!(validate(&incoming("m", "r", "u", "hi")).is_ok());
    }

    #[test]
    fn state_marks_only_the_message_being_judged() {
        let messages = [
            message(1, "r", "u1", "you're an idiot"),
            message(2, "r", "u2", "hello"),
            message(3, "r", "u3", "ok"),
        ];
        assert_eq!(
            worker::lines(&messages.iter().collect::<Vec<_>>(), 2),
            "u1: you're an idiot\n[this] u2: hello\nu3: ok"
        );
    }

    #[test]
    fn the_batch_is_applied_onto_the_room_in_order() {
        let prior = vec![message(10, "r", "u0", "earlier")];
        let batch = vec![
            message(1, "r", "u1", "you're an idiot"),
            message(2, "r", "u2", "hello"),
            message(3, "other", "u1", "ok"),
            message(4, "r", "u1", "idiot"),
        ];
        assert_eq!(
            worker::lines(&context(&prior, &batch, 0), 1),
            "u0: earlier\n[this] u1: you're an idiot"
        );
        assert_eq!(
            worker::lines(&context(&prior, &batch, 1), 2),
            "u0: earlier\nu1: you're an idiot\n[this] u2: hello"
        );
        assert_eq!(worker::lines(&context(&[], &batch, 2), 3), "[this] u1: ok");
        let fourth = context(&prior, &batch, 3);
        assert!(fourth.iter().any(|message| message.row_id == 1));
        assert!(fourth.iter().any(|message| message.row_id == 2));
        assert!(fourth.iter().all(|message| message.room_id == "r"));
    }

    #[test]
    fn a_repeated_client_id_is_another_message() {
        let prior = vec![message(1, "r", "u1", "old")];
        let batch = vec![
            message(2, "r", "u1", "first"),
            message(3, "r", "u1", "second"),
        ];
        let window = context(&prior, &batch, 1);
        assert_eq!(
            window
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            ["old", "first", "second"]
        );
    }

    #[test]
    fn the_batch_is_appended_in_request_order() {
        let prior = vec![message(10, "r", "u0", "stored")];
        let batch = vec![
            message(1, "r", "u1", "first"),
            message(2, "r", "u2", "second"),
        ];
        assert_eq!(
            worker::lines(&context(&prior, &batch, 1), 2),
            "u0: stored\nu1: first\n[this] u2: second"
        );
    }
}
