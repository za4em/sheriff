mod repository;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::category::{self, Action, Category};
use super::messages::IncomingMessage;
use super::provider::{DecisionProvider, Fault, Probabilities, Provider, Question, Questions};
use crate::error::AppError;

pub(crate) type CategoryScores = BTreeMap<Category, f64>;

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
    pub(crate) message_id: String,
    pub(crate) status: Status,
    pub(crate) action: Option<Action>,
    #[sqlx(json(nullable))]
    pub(crate) categories: Option<CategoryScores>,
}

#[derive(Clone)]
pub(crate) struct Decisions {
    pool: PgPool,
    provider: Provider,
    threshold: f64,
}

impl Decisions {
    pub(crate) fn new(pool: PgPool, provider: Provider, threshold: f64) -> Self {
        Self {
            pool,
            provider,
            threshold,
        }
    }

    pub(crate) async fn open(
        &self,
        tenant: &str,
        messages: &[IncomingMessage],
    ) -> Result<Vec<i64>, AppError> {
        repository::open(&self.pool, tenant, messages).await
    }

    pub(crate) async fn score(
        &self,
        message: &IncomingMessage,
        state: &str,
    ) -> Result<Decision, AppError> {
        let questions = message_questions();
        let probabilities = self
            .provider
            .decide(state, &questions)
            .await
            .map_err(AppError::Provider)?;
        let categories = category_scores(&probabilities).map_err(AppError::Provider)?;
        let chosen = category::action(&hits(&categories, self.threshold));
        Ok(Decision {
            message_id: message.message_id.clone(),
            status: Status::Ready,
            action: Some(chosen),
            categories: Some(categories),
        })
    }

    pub(crate) async fn write_ready(
        &self,
        decision_id: i64,
        decision: &Decision,
    ) -> Result<(), AppError> {
        let (Some(action), Some(categories)) = (decision.action, decision.categories.as_ref())
        else {
            return Ok(());
        };
        repository::write_ready(&self.pool, decision_id, action, categories).await
    }

    pub(crate) async fn mark_failed(&self, decision_id: i64) -> Result<(), AppError> {
        repository::mark_failed(&self.pool, decision_id).await
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
