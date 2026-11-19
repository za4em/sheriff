use tokio::sync::mpsc;

use super::Signal;
use super::log_background;
use crate::core::decisions::{Decision, Decisions, Status};
use crate::core::messages::{IncomingMessage, Messages};
use crate::core::provider::JUDGED;
use crate::core::queue::{Queue, ScoreJob};
use crate::error::{AppError, queue};

pub(super) async fn work(
    queue: Queue,
    decisions: Decisions,
    messages: Messages,
    signals: mpsc::UnboundedSender<Signal>,
) {
    loop {
        let incoming = match queue.recv().await {
            Ok(incoming) => incoming,
            Err(err) => {
                tracing::error!(error = %err, "score worker stopped");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };
        if let Err(err) = run(&incoming.job, &decisions, &messages, &signals).await {
            log_background(&err);
            let _ = incoming.nack().await;
            if signals.is_closed() {
                return;
            }
            continue;
        }
        if let Err(err) = incoming.ack().await {
            tracing::error!(error = %err, "score job could not be acknowledged");
        }
    }
}

async fn run(
    job: &ScoreJob,
    decisions: &Decisions,
    messages: &Messages,
    signals: &mpsc::UnboundedSender<Signal>,
) -> Result<(), AppError> {
    let Some(decision) = decisions.get(job.decision_id).await? else {
        signal(signals, job.decision_id, false)?;
        return Ok(());
    };
    if decision.status != Status::Pending {
        signal(signals, decision.id, false)?;
        return Ok(());
    }
    let fresh = score_message(decisions, messages, &decision.tenant, &decision).await?;
    signal(signals, decision.id, fresh)?;
    Ok(())
}

async fn score_message(
    decisions: &Decisions,
    messages: &Messages,
    tenant: &str,
    decision: &Decision,
) -> Result<bool, AppError> {
    let prompt = messages.get_many(tenant, &decision.context).await?;
    if prompt.len() != decision.context.len() {
        return decisions
            .mark_failed(decision.id, "The message could not be read.")
            .await;
    }
    let Some(judged) = prompt
        .iter()
        .find(|message| message.id == decision.message_id)
    else {
        return decisions
            .mark_failed(decision.id, "The message could not be read.")
            .await;
    };
    match decisions
        .score(&lines(&prompt.iter().collect::<Vec<_>>(), judged.id))
        .await
    {
        Ok((action, categories)) => decisions.mark_ready(decision.id, action, &categories).await,
        Err(err) => decisions.mark_failed(decision.id, &err.to_string()).await,
    }
}

fn signal(
    signals: &mpsc::UnboundedSender<Signal>,
    decision_id: i64,
    fresh: bool,
) -> Result<(), AppError> {
    signals
        .send(Signal { decision_id, fresh })
        .map_err(|_| queue("signaling a decision", "the observer stopped"))
}

pub(super) fn lines(messages: &[&IncomingMessage], judged: i64) -> String {
    messages
        .iter()
        .map(|message| {
            let mark = if message.id == judged { JUDGED } else { "" };
            format!("{mark}{}: {}", message.user_id, message.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
