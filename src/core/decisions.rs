mod repository;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::messages::IncomingMessage;
use super::provider::{DecisionProvider, Fault, Probabilities, Provider, Question, Questions};
use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub(crate) enum Status {
    Pending,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub(crate) enum Action {
    Allow,
    Hide,
    Mute,
    Escalate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Category {
    Insult,
    Bullying,
    Grooming,
    Hate,
    Threat,
    Scam,
    IllegalActivity,
    Doxxing,
    Spam,
}

impl Category {
    const ALL: [Self; 9] = [
        Self::Insult,
        Self::Bullying,
        Self::Grooming,
        Self::Hate,
        Self::Threat,
        Self::Scam,
        Self::IllegalActivity,
        Self::Doxxing,
        Self::Spam,
    ];

    fn question(self) -> &'static str {
        match self {
            Self::Insult => {
                "Is the message marked [this] a direct attack on a person (name-calling or humiliation)? Profanity with no target does not count. Other lines are context."
            }
            Self::Bullying => {
                "Given this chat, is the author of the message marked [this] bullying someone? A single insult does not count. Other lines are context."
            }
            Self::Grooming => {
                "Given this chat, is the author of the message marked [this] grooming someone (trust, secrecy, gifts, other apps, or photos with a likely minor)? One friendly message does not count. Other lines are context."
            }
            Self::Hate => {
                "Does the message marked [this] attack someone based on a protected group (ethnicity, religion, gender, orientation, or disability)? Other lines are context."
            }
            Self::Threat => {
                "Does the message marked [this] threaten violence or harm, including finding someone or swatting? Other lines are context."
            }
            Self::Scam => {
                "Is the message marked [this] a phishing link, fake giveaway, or a request for credentials or codes? Other lines are context."
            }
            Self::IllegalActivity => {
                "Is the message marked [this] selling or soliciting drugs, weapons, stolen accounts, cheats, fake documents, or a call to commit a crime? Other lines are context."
            }
            Self::Doxxing => {
                "Does the message marked [this] share someone else's personal information, or threaten to? Other lines are context."
            }
            Self::Spam => {
                "Is the message marked [this] advertising, self-promotion, a referral link, or an invite to another server? Other lines are context."
            }
        }
    }

    fn key(self) -> String {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::String(name)) => name,
            other => panic!("a category serializes as a string, got {other:?}"),
        }
    }

    fn parse(name: &str) -> Result<Self, AppError> {
        Self::ALL
            .into_iter()
            .find(|category| category.key() == name)
            .ok_or(AppError::UnknownCategory)
    }
}

pub(crate) type CategoryScores = BTreeMap<Category, f64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub(crate) enum DecidedBy {
    Rules,
    Model,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub(crate) struct Decision {
    pub(crate) message_id: String,
    pub(crate) status: Status,
    pub(crate) action: Option<Action>,
    #[sqlx(json(nullable))]
    pub(crate) categories: Option<CategoryScores>,
    pub(crate) decided_by: Option<DecidedBy>,
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
        let chosen = action(&hits(&categories, self.threshold));
        repository::write_ready(&self.pool, tenant, message, chosen, &categories).await?;
        Ok(Decision {
            message_id: message.message_id.clone(),
            status: Status::Ready,
            action: Some(chosen),
            categories: Some(categories),
            decided_by: Some(DecidedBy::Model),
        })
    }

    pub(crate) async fn mark_pending(
        &self,
        tenant: &str,
        message: &IncomingMessage,
    ) -> Result<(), AppError> {
        repository::mark_pending(&self.pool, tenant, message).await
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

fn action(hits: &[Category]) -> Action {
    if hits.is_empty() {
        Action::Allow
    } else if hits.iter().any(escalates) {
        Action::Escalate
    } else if hits.iter().any(mutes) {
        Action::Mute
    } else {
        Action::Hide
    }
}

fn escalates(category: &Category) -> bool {
    matches!(
        category,
        Category::Grooming | Category::Threat | Category::IllegalActivity
    )
}

fn mutes(category: &Category) -> bool {
    matches!(
        category,
        Category::Bullying | Category::Spam | Category::Scam
    )
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
        assert_eq!(action(&hits(&scores, THRESHOLD)), Action::Hide);
        scores.insert(Category::Insult, THRESHOLD - 0.01);
        assert_eq!(action(&hits(&scores, THRESHOLD)), Action::Allow);
    }

    #[test]
    fn each_hit_category_chooses_its_action() {
        for (category, expected) in [
            (Category::Insult, Action::Hide),
            (Category::Bullying, Action::Mute),
            (Category::Grooming, Action::Escalate),
            (Category::Hate, Action::Hide),
            (Category::Threat, Action::Escalate),
            (Category::Scam, Action::Mute),
            (Category::IllegalActivity, Action::Escalate),
            (Category::Doxxing, Action::Hide),
            (Category::Spam, Action::Mute),
        ] {
            let mut scores = scores();
            scores.insert(category, THRESHOLD);
            assert_eq!(action(&hits(&scores, THRESHOLD)), expected);
        }
    }

    #[test]
    fn the_stronger_action_wins() {
        let mut scores = scores();
        scores.insert(Category::Doxxing, THRESHOLD);
        scores.insert(Category::Scam, THRESHOLD);
        assert_eq!(action(&hits(&scores, THRESHOLD)), Action::Mute);

        scores.insert(Category::Threat, THRESHOLD);
        assert_eq!(action(&hits(&scores, THRESHOLD)), Action::Escalate);
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
    fn a_pending_decision_has_no_verdict() {
        let decision = Decision {
            message_id: "m1".to_owned(),
            status: Status::Pending,
            action: None,
            categories: None,
            decided_by: None,
        };
        let value = serde_json::to_value(&decision).unwrap();
        assert_eq!(value["status"], "pending");
        assert!(value["action"].is_null());
        assert!(value["categories"].is_null());
        assert!(value["decided_by"].is_null());
        assert!(value.get("triggered").is_none());
        assert!(value.get("latency_ms").is_none());
    }

    #[test]
    fn an_unknown_category_name_is_rejected() {
        assert!(Category::parse("bullying").is_ok());
        assert_eq!(
            Category::parse("harassment").unwrap_err(),
            AppError::UnknownCategory
        );
    }
}
