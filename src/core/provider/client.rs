use std::{collections::BTreeMap, sync::Arc, time::Duration};

use serde::Serialize;
use tokio::sync::Semaphore;

use super::{DecisionProvider, Fault, Probabilities, Questions};

#[derive(Clone)]
pub(crate) struct Client {
    http: reqwest::Client,
    model: String,
    url: String,
    api_key: String,
    limit: Arc<Semaphore>,
}

#[derive(Serialize)]
struct DecideRequest<'a> {
    model: &'a str,
    state: &'a str,
    questions: &'a Questions,
}

#[derive(serde::Deserialize)]
struct DecideResponse {
    questions: BTreeMap<String, Answer>,
}

#[derive(serde::Deserialize)]
struct Answer {
    probability: f64,
}

impl Client {
    pub(crate) fn new(
        url: String,
        model: String,
        api_key: String,
        timeout: Duration,
        max_concurrency: usize,
    ) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Self {
            http,
            model,
            url,
            api_key,
            limit: Arc::new(Semaphore::new(max_concurrency)),
        })
    }
}

impl DecisionProvider for Client {
    async fn decide(&self, state: &str, questions: &Questions) -> Result<Probabilities, Fault> {
        let _permit = self.limit.acquire().await.map_err(|err| {
            tracing::error!(error = %err, "decision concurrency limit closed");
            Fault::RequestRejected
        })?;

        let request = DecideRequest {
            model: &self.model,
            state,
            questions,
        };
        let response = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|err| {
                tracing::error!(error = %err, "decision request failed");
                Fault::RequestRejected
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            tracing::error!(error = %err, "decision response could not be read");
            Fault::BodyUnreadable
        })?;

        if !status.is_success() {
            log_snippet(&body);
            return Err(Fault::Upstream);
        }

        match probabilities(&body, questions) {
            Ok(probabilities) => Ok(probabilities),
            Err(fault) => {
                log_snippet(&body);
                Err(fault)
            }
        }
    }
}

pub(super) fn probabilities(body: &str, questions: &Questions) -> Result<Probabilities, Fault> {
    let response: DecideResponse = serde_json::from_str(body).map_err(|err| {
        tracing::error!(error = %err, "decision response was not probabilities");
        Fault::InvalidResponse
    })?;

    let mut scores = Probabilities::new();
    for key in questions.keys() {
        let Some(answer) = response.questions.get(key) else {
            return Err(Fault::InvalidResponse);
        };
        if !answer.probability.is_finite() || !(0.0..=1.0).contains(&answer.probability) {
            return Err(Fault::InvalidResponse);
        }
        scores.insert(key.clone(), answer.probability);
    }
    Ok(scores)
}

fn log_snippet(body: &str) {
    let snippet: String = body.chars().take(300).collect();
    tracing::error!(body = %snippet, "decision response was not probabilities");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::provider::Question;

    fn questions() -> Questions {
        let mut questions = Questions::new();
        questions.insert("insult".to_owned(), Question::score("Is this an insult?"));
        questions.insert("hate".to_owned(), Question::score("Is this hate?"));
        questions
    }

    #[test]
    fn request_names_model_state_and_questions() {
        let questions = questions();
        let body = serde_json::to_value(DecideRequest {
            model: "m",
            state: "hello",
            questions: &questions,
        })
        .unwrap();
        assert_eq!(body["model"], "m");
        assert_eq!(body["state"], "hello");
        assert_eq!(body["questions"]["insult"]["type"], "score");
        assert!(body["questions"]["insult"].get("options").is_none());
    }

    #[test]
    fn reads_a_probability_per_asked_question() {
        let body = r#"{"questions":{"insult":{"probability":0.91},"hate":{"probability":0.0}}}"#;
        let scores = probabilities(body, &questions()).unwrap();
        assert_eq!(scores["insult"], 0.91);
        assert_eq!(scores["hate"], 0.0);
    }

    #[test]
    fn a_missing_question_is_rejected() {
        let body = r#"{"questions":{"insult":{"probability":0.91}}}"#;
        assert_eq!(
            probabilities(body, &questions()).unwrap_err(),
            Fault::InvalidResponse
        );
    }

    #[test]
    fn a_probability_outside_zero_to_one_is_rejected() {
        let body = r#"{"questions":{"insult":{"probability":1.2},"hate":{"probability":0.0}}}"#;
        assert_eq!(
            probabilities(body, &questions()).unwrap_err(),
            Fault::InvalidResponse
        );
    }

    #[test]
    fn prose_is_rejected() {
        assert_eq!(
            probabilities("nope", &questions()).unwrap_err(),
            Fault::InvalidResponse
        );
    }
}
