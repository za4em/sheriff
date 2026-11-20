use std::time::Duration;

use crate::core::decisions::Decisions;
use crate::core::queue::{Queue, ScoreJob};
use crate::error::AppError;

const SWEEP_EVERY: Duration = Duration::from_secs(10);
const PENDING_AFTER: i64 = 30;
const SWEEP_LIMIT: i64 = 100;

/// Republishes decisions that are still pending after the provider should have finished.
pub(super) async fn run(decisions: Decisions, queue: Queue) {
    loop {
        if let Err(err) = republish(&decisions, &queue).await {
            tracing::error!(error = %err, "background work failed");
        }
        tokio::time::sleep(SWEEP_EVERY).await;
    }
}

async fn republish(decisions: &Decisions, queue: &Queue) -> Result<(), AppError> {
    let ids = decisions.stale_pending(PENDING_AFTER, SWEEP_LIMIT).await?;
    if ids.is_empty() {
        return Ok(());
    }
    tracing::info!(count = ids.len(), "republishing pending decisions");
    for id in ids {
        if let Err(err) = queue.publish(&ScoreJob { decision_id: id }).await {
            tracing::error!(error = %err, decision_id = id, "score job was not republished");
            break;
        }
    }
    Ok(())
}
