//! Tells every instance that a score job finished.
//!
//! The queue hands the job to one worker. The HTTP call or websocket waiting
//! for that decision stays on the instance that accepted it. Every instance
//! reads this stream and delivers the waiter it holds. A consumer group would
//! give each id to only one instance.

use redis::aio::ConnectionManager;

use crate::error::{AppError, cache as failed};

const READY: &str = "sheriff.ready";

/// One decision the worker has finished.
#[derive(Clone, Copy)]
pub(crate) struct ReadyDecision {
    pub(crate) decision_id: i64,
    /// True only for the worker that just finished the row.
    pub(crate) completed_now: bool,
}

/// Records ready decision ids and reads them on every instance.
///
/// The writer reconnects after the connection drops. The write in flight
/// still fails, and the worker requeues it. The reader keeps its own
/// connection so a blocking read does not stall writes.
#[derive(Clone)]
pub(crate) struct Cache {
    client: redis::Client,
    connection: ConnectionManager,
    limit: usize,
}

impl Cache {
    pub(crate) async fn connect(url: &str, limit: usize) -> Result<Self, AppError> {
        let client =
            redis::Client::open(url).map_err(|err| failed("connecting to the cache", err))?;
        let mut connection = ConnectionManager::new(client.clone())
            .await
            .map_err(|err| failed("connecting to the cache", err))?;
        let _: String = redis::cmd("PING")
            .query_async(&mut connection)
            .await
            .map_err(|err| failed("connecting to the cache", err))?;
        tracing::info!("connected to the cache");
        Ok(Self {
            client,
            connection,
            limit,
        })
    }

    pub(crate) async fn ready_decision(
        &self,
        decision_id: i64,
        completed_now: bool,
    ) -> Result<(), AppError> {
        let completed_now = i64::from(u8::from(completed_now));
        let mut connection = self.connection.clone();
        let _: Option<String> = redis::AsyncCommands::xadd_maxlen(
            &mut connection,
            READY,
            redis::streams::StreamMaxlen::Approx(self.limit),
            "*",
            &[
                ("decision_id", decision_id),
                ("completed_now", completed_now),
            ],
        )
        .await
        .map_err(|err| failed("recording a ready decision", err))?;
        Ok(())
    }
}
