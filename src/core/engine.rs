use std::collections::BTreeMap;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use super::cache::{Cache, ReadyDecision};
use super::decisions::{Decision, Decisions};
use super::messages::{IncomingMessage, Message, Messages};
use super::provider::Provider;
use super::queue::{Queue, ScoreJob};
use super::webhooks::Webhooks;
use crate::error::{AppError, database, queue};
use crate::metrics::Metrics;

mod plan;
mod sweep;
mod wait;
mod worker;

use plan::{BatchConfig, plan_windows};
use wait::{ScoreWait, WaitList, Waiter};

pub(crate) type SocketSender = mpsc::UnboundedSender<Decision>;

const SCORE_WAIT: Duration = Duration::from_secs(5);
const QUEUE_DEPTH_EVERY: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct Engine {
    pool: PgPool,
    pub(crate) messages: Messages,
    pub(crate) decisions: Decisions,
    pub(crate) webhooks: Webhooks,
    queue: Queue,
    cache: Cache,
    waiting: WaitList,
    metrics: Metrics,
    batch: BatchConfig,
}

impl Engine {
    pub(crate) fn new(
        pool: PgPool,
        provider: Provider,
        window: i64,
        overlap: i64,
        threshold: f64,
        queue: Queue,
        cache: Cache,
        metrics: Metrics,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            pool: pool.clone(),
            messages: Messages::new(pool.clone(), window),
            decisions: Decisions::new(pool.clone(), provider, threshold, metrics.clone()),
            webhooks: Webhooks::new(pool, metrics.clone())?,
            queue,
            cache,
            waiting: WaitList::default(),
            metrics,
            batch: BatchConfig {
                window: window as usize,
                overlap: overlap as usize,
            },
        })
    }

    /// Workers score jobs. The observer delivers sockets and webhooks.
    /// The sweeper republishes pending decisions left behind by a crash.
    pub(crate) async fn serve(&self, workers: usize) {
        watch_queue_depth(self.queue.clone(), self.metrics.clone());
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
            let metrics = self.metrics.clone();
            tokio::spawn(
                async move { worker::work(jobs, decisions, messages, cache, metrics).await },
            );
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
        let waits: Vec<ScoreWait> = ids
            .iter()
            .copied()
            .map(|id| self.waiting.reply(id))
            .collect();
        let published = self.enqueue(&ids).await;
        if published == 0 {
            drop(waits);
            if let Err(err) = self.decisions.delete(&ids).await {
                tracing::error!(error = %err, "background work failed");
            }
            return Err(queue(
                "publishing a score job",
                "the broker did not accept the job",
            ));
        }
        self.wait_for(waits).await?;
        let decisions = self.decisions.get_many(&ids).await;
        if let Err(err) = self.decisions.delete(&ids).await {
            tracing::error!(error = %err, "background work failed");
        }
        decisions
    }

    /// Stores the batch as retained and publishes one job for the group. Returns before the worker finishes.
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
        for message in messages {
            validate(message)?;
        }
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let priors = room_history(&self.messages, tenant, messages).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| database("saving a batch", err))?;
        let mut stored = Vec::with_capacity(messages.len());
        for message in messages {
            stored.push(super::messages::insert(&mut tx, tenant, message, retained).await?);
        }
        let windows = planned(&priors, &stored, self.batch);
        let group_id = uuid::Uuid::new_v4().to_string();
        let row_ids: Vec<i64> = stored.iter().map(|message| message.row_id).collect();
        let ids = super::decisions::insert_pending(&mut tx, tenant, &group_id, &row_ids).await?;
        super::decisions::insert_windows(&mut tx, &group_id, &windows).await?;
        tx.commit()
            .await
            .map_err(|err| database("saving a batch", err))?;
        Ok(ids)
    }

    /// Publishes the group's job, keyed by the minimum decision id.
    /// A job that does not reach the broker stays pending for the sweeper.
    /// Returns 1 when the job was published and 0 when it was not.
    async fn enqueue(&self, ids: &[i64]) -> usize {
        let Some(decision_id) = ids.iter().copied().min() else {
            return 0;
        };
        match self.queue.publish(&ScoreJob { decision_id }).await {
            Ok(()) => 1,
            Err(err) => {
                tracing::error!(error = %err, decision_id, "score job was not published");
                0
            }
        }
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

fn watch_queue_depth(queue: Queue, metrics: Metrics) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(QUEUE_DEPTH_EVERY);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if let Ok(depth) = queue.depth().await {
                metrics.set_queue_depth(depth);
            }
        }
    });
}

async fn room_history(
    messages: &Messages,
    tenant: &str,
    batch: &[IncomingMessage],
) -> Result<BTreeMap<String, Vec<Message>>, AppError> {
    let mut priors = BTreeMap::new();
    for message in batch {
        if priors.contains_key(&message.room_id) {
            continue;
        }
        let history = messages.room(tenant, &message.room_id).await?;
        priors.insert(message.room_id.clone(), history);
    }
    Ok(priors)
}

fn planned(
    priors: &BTreeMap<String, Vec<Message>>,
    stored: &[Message],
    config: BatchConfig,
) -> Vec<Vec<i64>> {
    let mut windows = Vec::new();
    for (room, incoming) in incoming_by_room(stored) {
        let prior = priors.get(&room).map(Vec::as_slice).unwrap_or(&[]);
        for window in plan_windows(prior, &incoming, config) {
            windows.push(
                window
                    .iter()
                    .map(|message| message.row_id)
                    .collect::<Vec<_>>(),
            );
        }
    }
    windows
}

fn incoming_by_room(stored: &[Message]) -> Vec<(String, Vec<Message>)> {
    let mut rooms: Vec<(String, Vec<Message>)> = Vec::new();
    for message in stored {
        if let Some((_, messages)) = rooms.iter_mut().find(|(room, _)| room == &message.room_id) {
            messages.push(message.clone());
        } else {
            rooms.push((message.room_id.clone(), vec![message.clone()]));
        }
    }
    rooms
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
}
