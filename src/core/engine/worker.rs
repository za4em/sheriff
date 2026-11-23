use crate::core::cache::Cache;
use crate::core::category::{Action, Category};
use crate::core::decisions::{CategoryScores, Decision, Decisions, Status, WindowVerdict};
use crate::core::messages::{Message, Messages};
use crate::core::provider::JUDGED;
use crate::core::queue::{Jobs, REQUEUE_PAUSE, ScoreJob};
use crate::error::AppError;
use crate::metrics::Metrics;

pub(super) async fn work(
    jobs: Jobs,
    decisions: Decisions,
    messages: Messages,
    cache: Cache,
    metrics: Metrics,
) {
    loop {
        let Some(delivery) = jobs.recv().await else {
            return;
        };
        let decision_id = delivery.job.decision_id;
        match score_group(&delivery.job, &decisions, &messages, &cache).await {
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
                } else {
                    metrics.retried();
                }
            }
        }
    }
}

struct Open {
    id: i64,
    message_row_id: i64,
    pending: bool,
    completed_now: bool,
    published: bool,
    first_hit: Option<usize>,
    failure: Option<String>,
}

fn open_rows(group: Vec<Decision>) -> Vec<Open> {
    group
        .into_iter()
        .map(|decision| Open {
            id: decision.id,
            message_row_id: decision.message_row_id,
            pending: decision.status == Status::Pending,
            completed_now: false,
            published: false,
            first_hit: None,
            failure: None,
        })
        .collect()
}

async fn publish_saved(cache: &Cache, group: &[Decision]) -> Result<(), AppError> {
    for decision in group {
        cache.ready_decision(decision.id, false).await?;
    }
    Ok(())
}

async fn score_group(
    job: &ScoreJob,
    decisions: &Decisions,
    messages: &Messages,
    cache: &Cache,
) -> Result<(), AppError> {
    let Some(seed) = decisions.get(job.decision_id).await? else {
        cache.ready_decision(job.decision_id, false).await?;
        return Ok(());
    };
    let group = decisions.group(&seed.group_id).await?;
    if group
        .iter()
        .all(|decision| decision.status != Status::Pending)
    {
        return publish_saved(cache, &group).await;
    }
    let mut open = open_rows(group);
    let windows = decisions.windows(&seed.group_id).await?;
    let hits = classify_windows(
        decisions,
        messages,
        cache,
        &seed.tenant,
        &windows,
        &mut open,
    )
    .await?;
    finish(decisions, cache, &mut open, &hits).await?;
    publish_rest(cache, &mut open).await
}

async fn publish_rest(cache: &Cache, open: &mut [Open]) -> Result<(), AppError> {
    for row in open {
        if !row.published {
            tell(cache, row).await?;
        }
    }
    Ok(())
}

async fn tell(cache: &Cache, row: &mut Open) -> Result<(), AppError> {
    cache.ready_decision(row.id, row.completed_now).await?;
    row.published = true;
    Ok(())
}

async fn classify_windows(
    decisions: &Decisions,
    messages: &Messages,
    cache: &Cache,
    tenant: &str,
    windows: &[Vec<i64>],
    open: &mut [Open],
) -> Result<Vec<Vec<Message>>, AppError> {
    let mut hits = Vec::new();
    for window in windows {
        if !open
            .iter()
            .any(|row| row.pending && window.contains(&row.message_row_id))
        {
            continue;
        }
        classify_window(decisions, messages, cache, tenant, window, open, &mut hits).await?;
    }
    Ok(hits)
}

async fn classify_window(
    decisions: &Decisions,
    messages: &Messages,
    cache: &Cache,
    tenant: &str,
    window: &[i64],
    open: &mut [Open],
    hits: &mut Vec<Vec<Message>>,
) -> Result<(), AppError> {
    let loaded = messages.get_many(tenant, window).await?;
    if loaded.len() != window.len() {
        return fail_unread(decisions, cache, open, window).await;
    }
    match decisions.judge_window(&batch_lines(&loaded)).await {
        Ok(WindowVerdict::Clean) => Ok(()),
        Ok(WindowVerdict::Hit) => {
            note_first_hit(open, window, hits.len());
            hits.push(loaded);
            Ok(())
        }
        Err(err @ AppError::Provider(_)) => {
            note_failure(open, window, err.to_string());
            Ok(())
        }
        Err(err) => Err(err),
    }
}

async fn fail_unread(
    decisions: &Decisions,
    cache: &Cache,
    open: &mut [Open],
    window: &[i64],
) -> Result<(), AppError> {
    for row in open.iter_mut() {
        if row.pending && window.contains(&row.message_row_id) {
            row.completed_now = decisions
                .mark_failed(row.id, "The message could not be read.")
                .await?;
            row.pending = false;
            tell(cache, row).await?;
        }
    }
    Ok(())
}

fn note_first_hit(open: &mut [Open], window: &[i64], index: usize) {
    for row in open.iter_mut() {
        if row.pending && row.first_hit.is_none() && window.contains(&row.message_row_id) {
            row.first_hit = Some(index);
        }
    }
}

fn note_failure(open: &mut [Open], window: &[i64], message: String) {
    for row in open.iter_mut() {
        if row.pending && row.failure.is_none() && window.contains(&row.message_row_id) {
            row.failure = Some(message.clone());
        }
    }
}

async fn finish(
    decisions: &Decisions,
    cache: &Cache,
    open: &mut [Open],
    hits: &[Vec<Message>],
) -> Result<(), AppError> {
    for row in open.iter_mut() {
        if !row.pending {
            continue;
        }
        row.completed_now = settle(decisions, hits, row).await?;
        row.pending = false;
        tell(cache, row).await?;
    }
    Ok(())
}

async fn settle(
    decisions: &Decisions,
    hits: &[Vec<Message>],
    row: &Open,
) -> Result<bool, AppError> {
    if let Some(index) = row.first_hit {
        let refs: Vec<&Message> = hits[index].iter().collect();
        let state = lines(&refs, row.message_row_id);
        return match decisions.score(&state).await {
            Ok((action, categories)) => decisions.mark_ready(row.id, action, &categories).await,
            Err(err @ AppError::Provider(_)) => {
                decisions.mark_failed(row.id, &err.to_string()).await
            }
            Err(err) => Err(err),
        };
    }
    if let Some(failure) = row.failure.clone() {
        return decisions.mark_failed(row.id, &failure).await;
    }
    decisions
        .mark_ready(row.id, Action::Allow, &zero_scores())
        .await
}

fn zero_scores() -> CategoryScores {
    Category::ALL
        .into_iter()
        .map(|category| (category, 0.0))
        .collect()
}

fn batch_lines(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| format!("{}: {}", message.user_id, message.text))
        .collect::<Vec<_>>()
        .join("\n")
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
