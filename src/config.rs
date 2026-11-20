use std::{env, error::Error, time::Duration};

use crate::core::provider::ProviderConfig;

pub struct Config {
    pub server: ServerConfig,
    pub(crate) database: DatabaseConfig,
    pub(crate) provider: ProviderConfig,
    pub(crate) api_key: String,
    pub(crate) window: i64,
    pub(crate) threshold: f64,
    pub(crate) workers: usize,
    pub(crate) queue_url: String,
    pub(crate) cache_url: String,
    pub(crate) cache_limit: usize,
}

pub struct ServerConfig {
    pub bind_address: String,
}

pub(crate) struct DatabaseConfig {
    pub(crate) url: String,
    pub(crate) pool_size: u32,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let workers = workers()?;
        Ok(Self {
            server: ServerConfig {
                bind_address: "0.0.0.0:3000".to_owned(),
            },
            database: DatabaseConfig {
                url: required("DATABASE_URL")?,
                pool_size: 20,
            },
            provider: provider(workers)?,
            api_key: required("SHERIFF_API_KEY")?,
            window: window()?,
            threshold: threshold()?,
            workers,
            queue_url: required("QUEUE_URL")?,
            cache_url: required("CACHE_URL")?,
            cache_limit: cache_limit()?,
        })
    }
}

fn provider(workers: usize) -> Result<ProviderConfig, Box<dyn Error>> {
    match var_or("SHERIFF_PROVIDER", "mock").as_str() {
        "mock" => Ok(ProviderConfig::Mock {
            latency: millis("SHERIFF_PROVIDER_LATENCY_MS", 0)?,
        }),
        "decision_api" => {
            let timeout = millis("SHERIFF_PROVIDER_TIMEOUT_MS", 300)?;
            if timeout.is_zero() {
                return Err("SHERIFF_PROVIDER_TIMEOUT_MS must be greater than 0".into());
            }
            Ok(ProviderConfig::Api {
                base_url: required("SHERIFF_PROVIDER_BASE_URL")?,
                model: required("SHERIFF_PROVIDER_MODEL")?,
                api_key: required("SHERIFF_PROVIDER_API_KEY")?,
                timeout,
                max_concurrency: workers,
            })
        }
        other => Err(format!("SHERIFF_PROVIDER must be mock or decision_api, not {other}").into()),
    }
}

fn threshold() -> Result<f64, Box<dyn Error>> {
    let raw = var_or("SHERIFF_SCORE_THRESHOLD", "0.8");
    let value: f64 = raw
        .parse()
        .map_err(|_| "SHERIFF_SCORE_THRESHOLD must be a number".to_owned())?;
    if !value.is_finite() || value <= 0.0 || value > 1.0 {
        return Err("SHERIFF_SCORE_THRESHOLD must be greater than 0 and at most 1".into());
    }
    Ok(value)
}

fn window() -> Result<i64, Box<dyn Error>> {
    let raw = var_or("SHERIFF_MESSAGES_WINDOW", "50");
    let value: i64 = raw
        .parse()
        .map_err(|_| "SHERIFF_MESSAGES_WINDOW must be a number".to_owned())?;
    if value <= 0 {
        return Err("SHERIFF_MESSAGES_WINDOW must be greater than 0".into());
    }
    Ok(value)
}

fn cache_limit() -> Result<usize, Box<dyn Error>> {
    let raw = var_or("SHERIFF_CACHE_LIMIT", "100000");
    let value: usize = raw
        .parse()
        .map_err(|_| "SHERIFF_CACHE_LIMIT must be a number".to_owned())?;
    if value == 0 {
        return Err("SHERIFF_CACHE_LIMIT must be greater than 0".into());
    }
    Ok(value)
}

fn workers() -> Result<usize, Box<dyn Error>> {
    let raw = var_or("SHERIFF_WORKERS", "100");
    let value: usize = raw
        .parse()
        .map_err(|_| "SHERIFF_WORKERS must be a number".to_owned())?;
    if value == 0 {
        return Err("SHERIFF_WORKERS must be greater than 0".into());
    }
    Ok(value)
}

fn millis(name: &str, default: u64) -> Result<Duration, Box<dyn Error>> {
    let raw = var_or(name, &default.to_string());
    let value: u64 = raw
        .parse()
        .map_err(|_| format!("{name} must be a number of milliseconds"))?;
    Ok(Duration::from_millis(value))
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn required(name: &str) -> Result<String, Box<dyn Error>> {
    let value = env::var(name).map_err(|_| format!("{name} is required"))?;
    if value.trim().is_empty() {
        return Err(format!("{name} is required").into());
    }
    Ok(value)
}
