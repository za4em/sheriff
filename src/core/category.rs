use serde::{Deserialize, Serialize};

use crate::error::AppError;

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
    pub(crate) const ALL: [Self; 9] = [
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

    pub(crate) fn question(self) -> &'static str {
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

    pub(crate) fn key(self) -> String {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::String(name)) => name,
            other => panic!("a category serializes as a string, got {other:?}"),
        }
    }

    pub(crate) fn parse(name: &str) -> Result<Self, AppError> {
        Self::ALL
            .into_iter()
            .find(|category| category.key() == name)
            .ok_or(AppError::UnknownCategory)
    }
}

pub(crate) fn action(hits: &[Category]) -> Action {
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
            assert_eq!(action(&[category]), expected);
        }
    }

    #[test]
    fn the_stronger_action_wins() {
        assert_eq!(action(&[Category::Doxxing, Category::Scam]), Action::Mute);
        assert_eq!(
            action(&[Category::Doxxing, Category::Scam, Category::Threat]),
            Action::Escalate
        );
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
