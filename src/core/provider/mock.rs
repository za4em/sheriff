use std::time::Duration;

use super::{DecisionProvider, Fault, Probabilities, Questions};

pub(crate) const INSULT_SCORE: f64 = 0.95;

#[derive(Clone)]
pub(crate) struct Mock {
    latency: Duration,
}

impl Mock {
    pub(crate) fn new(latency: Duration) -> Self {
        Self { latency }
    }
}

impl DecisionProvider for Mock {
    async fn decide(&self, state: &str, questions: &Questions) -> Result<Probabilities, Fault> {
        if !self.latency.is_zero() {
            tokio::time::sleep(self.latency).await;
        }
        let insult = if judged_has_word(state, "idiot") {
            INSULT_SCORE
        } else {
            0.0
        };
        let bullying = if author_repeats_word(state, "idiot") {
            INSULT_SCORE
        } else {
            0.0
        };
        let grooming = if judged_has_word(state, "secret") && author_has_other_line(state) {
            INSULT_SCORE
        } else {
            0.0
        };
        Ok(questions
            .keys()
            .map(|key| {
                let score = match key.as_str() {
                    "insult" => insult,
                    "bullying" => bullying,
                    "grooming" => grooming,
                    _ => 0.0,
                };
                (key.clone(), score)
            })
            .collect())
    }
}

fn author_has_other_line(state: &str) -> bool {
    let Some(author) = judged_author(state) else {
        return false;
    };
    state
        .lines()
        .filter(|line| line_author(line) == Some(author))
        .count()
        >= 2
}

fn judged_has_word(state: &str, word: &str) -> bool {
    state.lines().any(|line| {
        let Some(text) = line.strip_prefix(super::JUDGED) else {
            return false;
        };
        has_word(text, word)
    })
}

fn author_repeats_word(state: &str, word: &str) -> bool {
    let Some(author) = judged_author(state) else {
        return false;
    };
    state
        .lines()
        .filter(|line| line_author(line) == Some(author) && has_word(line, word))
        .count()
        >= 2
}

fn judged_author(state: &str) -> Option<&str> {
    state
        .lines()
        .find_map(|line| line.strip_prefix(super::JUDGED).and_then(line_author))
}

fn line_author(line: &str) -> Option<&str> {
    let line = line.strip_prefix(super::JUDGED).unwrap_or(line);
    let author = line.split(':').next()?.trim();
    if author.is_empty() {
        None
    } else {
        Some(author)
    }
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|ch: char| !ch.is_alphanumeric())
        .any(|part| part.eq_ignore_ascii_case(word))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::provider::Question;

    fn questions() -> Questions {
        let mut questions = Questions::new();
        questions.insert("insult".to_owned(), Question::score("insult"));
        questions.insert("bullying".to_owned(), Question::score("bullying"));
        questions.insert("grooming".to_owned(), Question::score("grooming"));
        questions.insert("spam".to_owned(), Question::score("spam"));
        questions
    }

    #[tokio::test]
    async fn idiot_is_the_canned_insult() {
        let scores = Mock::new(Duration::ZERO)
            .decide("[this] u: you Idiot", &questions())
            .await
            .unwrap();
        assert_eq!(scores["insult"], INSULT_SCORE);
        assert_eq!(scores["bullying"], 0.0);
        assert_eq!(scores["grooming"], 0.0);
        assert_eq!(scores["spam"], 0.0);
    }

    #[tokio::test]
    async fn a_word_that_only_contains_the_marker_is_not_an_insult() {
        let scores = Mock::new(Duration::ZERO)
            .decide("[this] u: idiotic", &questions())
            .await
            .unwrap();
        assert_eq!(scores["insult"], 0.0);
    }

    #[tokio::test]
    async fn an_insult_on_another_line_is_not_this_message() {
        let scores = Mock::new(Duration::ZERO)
            .decide(
                "u1: you're an idiot\n[this] u2: hello\nu3: ok",
                &questions(),
            )
            .await
            .unwrap();
        assert_eq!(scores["insult"], 0.0);
        assert_eq!(scores["bullying"], 0.0);
        assert_eq!(scores["grooming"], 0.0);
    }

    #[tokio::test]
    async fn a_repeated_insult_from_the_author_is_bullying() {
        let scores = Mock::new(Duration::ZERO)
            .decide(
                "u1: you're an idiot\nu2: hello\n[this] u1: idiot",
                &questions(),
            )
            .await
            .unwrap();
        assert_eq!(scores["insult"], INSULT_SCORE);
        assert_eq!(scores["bullying"], INSULT_SCORE);
        assert_eq!(scores["grooming"], 0.0);
    }

    #[tokio::test]
    async fn one_secret_line_is_not_grooming() {
        let scores = Mock::new(Duration::ZERO)
            .decide("[this] u: keep this secret", &questions())
            .await
            .unwrap();
        assert_eq!(scores["grooming"], 0.0);
    }

    #[tokio::test]
    async fn secrecy_after_another_line_from_the_author_is_grooming() {
        let scores = Mock::new(Duration::ZERO)
            .decide("u1: hey\n[this] u1: keep this secret", &questions())
            .await
            .unwrap();
        assert_eq!(scores["grooming"], INSULT_SCORE);
        assert_eq!(scores["insult"], 0.0);
        assert_eq!(scores["bullying"], 0.0);
    }
}
