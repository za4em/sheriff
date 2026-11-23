mod repository;

use std::collections::BTreeMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::category::{self, Action, Category};
use super::provider::{DecisionProvider, Fault, Probabilities, Provider, Question, Questions};
use crate::error::AppError;
use crate::metrics::Metrics;

const FAILED: &str = "failed";

pub(crate) type CategoryScores = BTreeMap<Category, f64>;

pub(crate) enum Screen {
    Clean,
    Hit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub(crate) enum Status {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub(crate) struct Decision {
    #[serde(skip_serializing)]
    pub(crate) id: i64,
    #[serde(skip_serializing)]
    pub(crate) group_id: String,
    #[serde(skip_serializing)]
    #[sqlx(rename = "tenant_id")]
    pub(crate) tenant: String,
    #[serde(skip_serializing)]
    #[sqlx(rename = "message_id")]
    pub(crate) message_row_id: i64,
    #[serde(rename = "message_id")]
    pub(crate) client_message_id: String,
    pub(crate) status: Status,
    pub(crate) action: Option<Action>,
    #[sqlx(json(nullable))]
    pub(crate) categories: Option<CategoryScores>,
    pub(crate) error: Option<String>,
}

pub(crate) use repository::{insert_pending, insert_windows};

#[derive(Clone)]
pub(crate) struct Decisions {
    pool: PgPool,
    provider: Provider,
    threshold: f64,
    metrics: Metrics,
}

impl Decisions {
    pub(crate) fn new(pool: PgPool, provider: Provider, threshold: f64, metrics: Metrics) -> Self {
        for action in [Action::Allow, Action::Hide, Action::Mute, Action::Escalate] {
            metrics.prepare_decision(action.name());
        }
        metrics.prepare_decision(FAILED);
        Self {
            pool,
            provider,
            threshold,
            metrics,
        }
    }

    pub(crate) async fn stale_pending(
        &self,
        seconds: i64,
        limit: i64,
    ) -> Result<Vec<i64>, AppError> {
        repository::stale_pending(&self.pool, seconds, limit).await
    }

    pub(crate) async fn group(&self, group_id: &str) -> Result<Vec<Decision>, AppError> {
        repository::group(&self.pool, group_id).await
    }

    pub(crate) async fn windows(&self, group_id: &str) -> Result<Vec<Vec<i64>>, AppError> {
        repository::windows(&self.pool, group_id).await
    }

    pub(crate) async fn score(&self, state: &str) -> Result<(Action, CategoryScores), AppError> {
        let questions = message_questions();
        let started = Instant::now();
        let decided = self.provider.decide(state, &questions).await;
        self.metrics.observe_provider(started.elapsed());
        let probabilities = decided.map_err(AppError::Provider)?;
        let categories = category_scores(&probabilities).map_err(AppError::Provider)?;
        let chosen = category::action(&hits(&categories, self.threshold));
        Ok((chosen, categories))
    }

    pub(crate) async fn screen(&self, state: &str) -> Result<Screen, AppError> {
        let questions = batch_questions();
        let started = Instant::now();
        let decided = self.provider.decide(state, &questions).await;
        self.metrics.observe_provider(started.elapsed());
        let probabilities = decided.map_err(AppError::Provider)?;
        let categories = category_scores(&probabilities).map_err(AppError::Provider)?;
        if hits(&categories, self.threshold).is_empty() {
            Ok(Screen::Clean)
        } else {
            Ok(Screen::Hit)
        }
    }

    pub(crate) async fn mark_ready(
        &self,
        decision_id: i64,
        action: Action,
        categories: &CategoryScores,
    ) -> Result<bool, AppError> {
        let saved = repository::mark_ready(&self.pool, decision_id, action, categories).await?;
        if saved {
            self.metrics.decision(action.name());
        }
        Ok(saved)
    }

    pub(crate) async fn mark_failed(
        &self,
        decision_id: i64,
        error: &str,
    ) -> Result<bool, AppError> {
        let saved = repository::mark_failed(&self.pool, decision_id, error).await?;
        if saved {
            self.metrics.decision(FAILED);
        }
        Ok(saved)
    }

    pub(crate) async fn get(&self, id: i64) -> Result<Option<Decision>, AppError> {
        Ok(self.get_many(&[id]).await?.into_iter().next())
    }

    pub(crate) async fn get_many(&self, ids: &[i64]) -> Result<Vec<Decision>, AppError> {
        repository::get(&self.pool, ids).await
    }

    pub(crate) async fn delete(&self, ids: &[i64]) -> Result<(), AppError> {
        repository::delete(&self.pool, ids).await
    }

    pub(crate) async fn list(
        &self,
        tenant: &str,
        room: &str,
        ids: &[String],
        user_id: Option<&str>,
        after: Option<i64>,
        category: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Decision>, AppError> {
        let category = category.map(Category::parse).transpose()?;
        if room.trim().is_empty() {
            return Ok(Vec::new());
        }
        repository::list(
            &self.pool,
            tenant,
            room,
            ids,
            user_id,
            after,
            category.map(Category::key),
            self.threshold,
            &category_names(),
            limit,
        )
        .await
    }
}

fn category_names() -> Vec<String> {
    Category::ALL.into_iter().map(Category::key).collect()
}

fn message_questions() -> Questions {
    Category::ALL
        .into_iter()
        .map(|category| (category.key(), Question::score(category.question())))
        .collect()
}

fn batch_questions() -> Questions {
    Category::ALL
        .into_iter()
        .map(|category| (category.key(), Question::score(category.batch_question())))
        .collect()
}

fn category_scores(probabilities: &Probabilities) -> Result<CategoryScores, Fault> {
    let mut scores = CategoryScores::new();
    for category in Category::ALL {
        let key = category.key();
        let Some(score) = probabilities.get(&key).copied() else {
            tracing::error!(category = %key, "decision scores were missing a category");
            return Err(Fault::InvalidResponse);
        };
        scores.insert(category, score);
    }
    Ok(scores)
}

fn hits(scores: &CategoryScores, threshold: f64) -> Vec<Category> {
    Category::ALL
        .into_iter()
        .filter(|category| scores[category] >= threshold)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD: f64 = 0.8;

    fn scores() -> CategoryScores {
        Category::ALL
            .into_iter()
            .map(|category| (category, 0.0))
            .collect()
    }

    #[test]
    fn threshold_is_inclusive() {
        let mut scores = scores();
        scores.insert(Category::Insult, THRESHOLD);
        assert_eq!(category::action(&hits(&scores, THRESHOLD)), Action::Hide);
        scores.insert(Category::Insult, THRESHOLD - 0.01);
        assert_eq!(category::action(&hits(&scores, THRESHOLD)), Action::Allow);
    }

    #[test]
    fn a_missing_category_is_rejected() {
        let complete = probabilities(None);
        assert!(category_scores(&complete).is_ok());

        let missing = probabilities(Some(Category::Doxxing));
        assert_eq!(
            category_scores(&missing).unwrap_err(),
            Fault::InvalidResponse
        );
    }

    fn probabilities(missing: Option<Category>) -> Probabilities {
        Category::ALL
            .into_iter()
            .filter(|category| Some(*category) != missing)
            .map(|category| (category.key(), 0.0))
            .collect()
    }
}
