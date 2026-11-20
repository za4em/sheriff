use tokio::sync::mpsc;

use super::FinishedDecision;
use crate::core::decisions::{Decision, Decisions, Status};
use crate::core::messages::{Message, Messages};
use crate::core::provider::JUDGED;
use crate::core::queue::{Jobs, REQUEUE_PAUSE, ScoreJob};
use crate::error::AppError;

pub(super) async fn work(
    jobs: Jobs,
    decisions: Decisions,
    messages: Messages,
    finished: mpsc::UnboundedSender<FinishedDecision>,
) {
    loop {
        let Some(delivery) = jobs.recv().await else {
            return;
        };
        let decision_id = delivery.job.decision_id;
        match run(&delivery.job, &decisions, &messages, &finished).await {
            Ok(()) => {
                if let Err(err) = delivery.ack().await {
                    tracing::error!(error = %err, decision_id, "score job could not be acknowledged");
                }
            }
            Err(err) => {
                tracing::error!(error = %err, decision_id, "score job failed");
                tokio::time::sleep(REQUEUE_PAUSE).await;
                if let Err(err) = delivery.requeue().await {
                    tracing::error!(error = %err, "score job could not be requeued");
                }
            }
        }
    }
}

async fn run(
    job: &ScoreJob,
    decisions: &Decisions,
    messages: &Messages,
    finished: &mpsc::UnboundedSender<FinishedDecision>,
) -> Result<(), AppError> {
    let Some(decision) = decisions.get(job.decision_id).await? else {
        notify(finished, job.decision_id, false);
        return Ok(());
    };
    if decision.status != Status::Pending {
        notify(finished, decision.id, false);
        return Ok(());
    }
    let completed_now = score_message(decisions, messages, &decision).await?;
    notify(finished, decision.id, completed_now);
    Ok(())
}

async fn score_message(
    decisions: &Decisions,
    messages: &Messages,
    decision: &Decision,
) -> Result<bool, AppError> {
    let prompt = messages
        .get_many(&decision.tenant, &decision.context)
        .await?;
    if prompt.len() != decision.context.len() {
        return decisions
            .mark_failed(decision.id, "The message could not be read.")
            .await;
    }
    let Some(judged) = prompt
        .iter()
        .find(|message| message.row_id == decision.message_row_id)
    else {
        return decisions
            .mark_failed(decision.id, "The message could not be read.")
            .await;
    };
    let lines = lines(&prompt.iter().collect::<Vec<_>>(), judged.row_id);
    match decisions.score(&lines).await {
        Ok((action, categories)) => decisions.mark_ready(decision.id, action, &categories).await,
        Err(err) => decisions.mark_failed(decision.id, &err.to_string()).await,
    }
}

fn notify(
    finished: &mpsc::UnboundedSender<FinishedDecision>,
    decision_id: i64,
    completed_now: bool,
) {
    if finished
        .send(FinishedDecision {
            decision_id,
            completed_now,
        })
        .is_err()
    {
        tracing::error!(
            decision_id,
            "the observer stopped before a decision was delivered"
        );
    }
}

pub(super) fn lines(messages: &[&Message], judged_row_id: i64) -> String {
    messages
        .iter()
        .map(|message| {
            let mark = if message.row_id == judged_row_id {
                JUDGED
            } else {
                ""
            };
            format!("{mark}{}: {}", message.user_id, message.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
