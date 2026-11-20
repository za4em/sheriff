//! Reads `sheriff.ready` on this instance.

use std::time::Duration;

use redis::streams::StreamReadOptions;
use tokio::sync::{mpsc, oneshot};

use super::{READY, ReadyDecision};

const READ_BLOCK_MS: usize = 5_000;
const READ_COUNT: usize = 100;
const RECONNECT_PAUSE: Duration = Duration::from_secs(1);
/// The crate aborts a command after 500ms. The reader waits past the server
/// block so a quiet stream can answer nil. A stall with no reply still fails
/// and opens a new connection.
const READ_TIMEOUT: Duration = Duration::from_millis(READ_BLOCK_MS as u64 + 5_000);

struct Cursor {
    last_id: String,
    started: bool,
}

impl Cursor {
    fn new() -> Self {
        Self {
            last_id: "0-0".to_owned(),
            started: false,
        }
    }

    async fn resume(
        &mut self,
        connection: &mut redis::aio::MultiplexedConnection,
        ready: &mut Option<oneshot::Sender<()>>,
    ) -> Result<(), redis::RedisError> {
        if self.started {
            tracing::info!("ready stream reconnected");
            return Ok(());
        }
        self.last_id = latest_id(connection).await?;
        self.started = true;
        if let Some(sender) = ready.take() {
            tracing::info!("listening for ready decisions");
            let _ = sender.send(());
        }
        Ok(())
    }
}

pub(super) async fn listen_decisions(
    client: redis::Client,
) -> mpsc::UnboundedReceiver<ReadyDecision> {
    let (tx, rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    tokio::spawn(async move { follow_decisions(client, tx, ready_tx).await });
    if ready_rx.await.is_err() {
        tracing::error!("ready stream stopped before it started");
    }
    rx
}

async fn follow_decisions(
    client: redis::Client,
    tx: mpsc::UnboundedSender<ReadyDecision>,
    ready: oneshot::Sender<()>,
) {
    let mut ready = Some(ready);
    let mut cursor = Cursor::new();
    loop {
        if tx.is_closed() {
            return;
        }
        match read_decisions(&client, &tx, &mut ready, &mut cursor).await {
            Ok(()) => {}
            Err(err) => tracing::error!(error = %err, "ready stream failed"),
        }
        tokio::time::sleep(RECONNECT_PAUSE).await;
    }
}

async fn read_decisions(
    client: &redis::Client,
    tx: &mpsc::UnboundedSender<ReadyDecision>,
    ready: &mut Option<oneshot::Sender<()>>,
    cursor: &mut Cursor,
) -> Result<(), redis::RedisError> {
    let mut connection = reader(client).await?;
    cursor.resume(&mut connection, ready).await?;
    loop {
        if tx.is_closed() {
            return Ok(());
        }
        let Some(reply) = batch(&mut connection, &cursor.last_id).await? else {
            continue;
        };
        for entry in entries(&reply) {
            cursor.last_id.clone_from(&entry.id);
            let Some(signal) = parse_ready_decision(entry) else {
                tracing::error!("ready decision could not be read");
                continue;
            };
            if tx.send(signal).is_err() {
                return Ok(());
            }
        }
    }
}

async fn reader(
    client: &redis::Client,
) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
    let config = redis::AsyncConnectionConfig::new().set_response_timeout(Some(READ_TIMEOUT));
    client
        .get_multiplexed_async_connection_with_config(&config)
        .await
}

/// An idle block is a nil reply: an empty read, not a dropped connection.
async fn batch(
    connection: &mut redis::aio::MultiplexedConnection,
    last_id: &str,
) -> Result<Option<redis::streams::StreamReadReply>, redis::RedisError> {
    let options = StreamReadOptions::default()
        .block(READ_BLOCK_MS)
        .count(READ_COUNT);
    redis::AsyncCommands::xread_options(connection, &[READY], &[last_id], &options).await
}

fn entries(
    reply: &redis::streams::StreamReadReply,
) -> impl Iterator<Item = &redis::streams::StreamId> {
    reply.keys.iter().flat_map(|key| key.ids.iter())
}

async fn latest_id(
    connection: &mut redis::aio::MultiplexedConnection,
) -> Result<String, redis::RedisError> {
    let reply: redis::streams::StreamRangeReply =
        redis::AsyncCommands::xrevrange_count(connection, READY, "+", "-", 1usize).await?;
    Ok(reply
        .ids
        .into_iter()
        .next()
        .map(|entry| entry.id)
        .unwrap_or_else(|| "0-0".to_owned()))
}

fn parse_ready_decision(entry: &redis::streams::StreamId) -> Option<ReadyDecision> {
    let decision_id = redis::from_redis_value_ref(entry.map.get("decision_id")?).ok()?;
    let completed_now = entry
        .map
        .get("completed_now")
        .and_then(|value| redis::from_redis_value_ref::<i64>(value).ok())
        .is_some_and(|value| value != 0);
    Some(ReadyDecision {
        decision_id,
        completed_now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ready_entry_carries_the_decision_id() {
        let mut entry = redis::streams::StreamId::default();
        entry
            .map
            .insert("decision_id".to_owned(), redis::Value::Int(7));
        entry
            .map
            .insert("completed_now".to_owned(), redis::Value::Int(1));
        assert_eq!(
            parse_ready_decision(&entry)
                .map(|decision| (decision.decision_id, decision.completed_now)),
            Some((7, true))
        );
        entry.map.insert(
            "decision_id".to_owned(),
            redis::Value::BulkString(b"8".to_vec()),
        );
        entry.map.insert(
            "completed_now".to_owned(),
            redis::Value::BulkString(b"0".to_vec()),
        );
        assert_eq!(
            parse_ready_decision(&entry)
                .map(|decision| (decision.decision_id, decision.completed_now)),
            Some((8, false))
        );
    }
}
