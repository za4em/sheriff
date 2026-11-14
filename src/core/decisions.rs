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

    pub(crate) async fn decide(
        &self,
        tenant: &str,
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
        repository::write_ready(&self.pool, tenant, message, chosen, &categories).await?;
        Ok(Decision {
            message_id: message.message_id.clone(),
            status: Status::Ready,
            action: Some(chosen),
            categories: Some(categories),
        })
    }

    pub(crate) async fn mark_pending(
        &self,
        tenant: &str,
        message: &IncomingMessage,
    ) -> Result<(), AppError> {
        repository::mark_pending(&self.pool, tenant, message).await
    }

    pub(crate) async fn mark_failed(
        &self,
        tenant: &str,
        message: &IncomingMessage,
    ) -> Result<(), AppError> {
        repository::mark_failed(&self.pool, tenant, message).await
    }

    pub(crate) async fn list(
        &self,
        tenant: &str,
        room: &str,
        ids: &[String],
        user_id: Option<&str>,
        after: Option<i64>,
        category: Option<&str>,
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
        )
        .await
    }
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
    fn each_category_is_one_score_question() {
        let questions = message_questions();
        assert_eq!(questions.len(), Category::ALL.len());
        let insult = serde_json::to_value(&questions[&Category::Insult.key()]).unwrap();
        assert_eq!(insult["type"], "score");
        assert_eq!(insult["instructions"], Category::Insult.question());
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

    #[test]
    fn an_unfinished_decision_has_no_verdict() {
        for (status, name) in [(Status::Pending, "pending"), (Status::Failed, "failed")] {
            let decision = Decision {
                message_id: "m1".to_owned(),
                status,
                action: None,
                categories: None,
            };
            let value = serde_json::to_value(&decision).unwrap();
            assert_eq!(value["status"], name);
            assert!(value["action"].is_null());
            assert!(value["categories"].is_null());
            assert!(value.get("decided_by").is_none());
            assert!(value.get("triggered").is_none());
            assert!(value.get("latency_ms").is_none());
        }
    }
}
