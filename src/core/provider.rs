mod client;
mod mock;

use std::{collections::BTreeMap, error::Error, future::Future, time::Duration};

use client::Client;
use serde::Serialize;

pub(crate) use mock::Mock;

pub(crate) const JUDGED: &str = "[this] ";

pub(crate) type Questions = BTreeMap<String, Question>;
pub(crate) type Probabilities = BTreeMap<String, f64>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fault {
    RequestRejected,
    BodyUnreadable,
    Upstream,
    InvalidResponse,
}

pub(crate) enum ProviderConfig {
    Mock {
        latency: Duration,
    },
    Api {
        base_url: String,
        model: String,
        api_key: String,
        timeout: Duration,
        max_concurrency: usize,
    },
}

#[derive(Clone)]
pub(crate) enum Provider {
    Client(Client),
    Mock(Mock),
}

impl Provider {
    pub(crate) fn from_config(config: ProviderConfig) -> Result<Self, Box<dyn Error>> {
        Ok(match config {
            ProviderConfig::Mock { latency } => Self::Mock(Mock::new(latency)),
            ProviderConfig::Api {
                base_url,
                model,
                api_key,
                timeout,
                max_concurrency,
            } => Self::Client(Client::new(
                base_url,
                model,
                api_key,
                timeout,
                max_concurrency,
            )?),
        })
    }
}

impl DecisionProvider for Provider {
    async fn decide(&self, state: &str, questions: &Questions) -> Result<Probabilities, Fault> {
        match self {
            Self::Client(client) => client.decide(state, questions).await,
            Self::Mock(mock) => mock.decide(state, questions).await,
        }
    }
}

pub(crate) trait DecisionProvider {
    fn decide(
        &self,
        state: &str,
        questions: &Questions,
    ) -> impl Future<Output = Result<Probabilities, Fault>> + Send;
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Question {
    #[serde(rename = "type")]
    kind: QuestionKind,
    instructions: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    options: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum QuestionKind {
    Score,
    #[allow(dead_code)]
    Choice,
}

impl Question {
    pub(crate) fn score(instructions: impl Into<String>) -> Self {
        Self {
            kind: QuestionKind::Score,
            instructions: instructions.into(),
            options: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn choice(instructions: impl Into<String>, options: Vec<String>) -> Self {
        Self {
            kind: QuestionKind::Choice,
            instructions: instructions.into(),
            options,
        }
    }
}
