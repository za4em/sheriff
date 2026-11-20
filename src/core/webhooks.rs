mod repository;

use std::time::Duration;

use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

use super::decisions::Decision;
use crate::error::AppError;
use crate::metrics::Metrics;

const SIGNATURE: &str = "X-Sheriff-Signature";
const TIMEOUT: Duration = Duration::from_secs(5);

type Signer = Hmac<Sha256>;

#[derive(Debug, Serialize)]
pub(crate) struct RegisteredEndpoint {
    pub(crate) id: i64,
    pub(crate) url: String,
    pub(crate) secret: String,
}

#[derive(Clone)]
pub(crate) struct Webhooks {
    pool: PgPool,
    http: reqwest::Client,
    metrics: Metrics,
}

impl Webhooks {
    pub(crate) fn new(pool: PgPool, metrics: Metrics) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder().timeout(TIMEOUT).build()?;
        Ok(Self {
            pool,
            http,
            metrics,
        })
    }

    pub(crate) async fn register(
        &self,
        tenant: &str,
        url: &str,
    ) -> Result<RegisteredEndpoint, AppError> {
        let url = require_url(url)?;
        let secret = Uuid::new_v4().simple().to_string();
        let id = repository::insert(&self.pool, tenant, &url, &secret).await?;
        Ok(RegisteredEndpoint { id, url, secret })
    }

    pub(crate) async fn deliver_decision(
        &self,
        tenant: &str,
        decision: &Decision,
    ) -> Result<(), AppError> {
        let endpoints = repository::endpoints(&self.pool, tenant).await?;
        let body = serde_json::to_vec(decision).map_err(|err| {
            tracing::error!(error = %err, "webhook body could not be encoded");
            AppError::WebhookEncode
        })?;
        for endpoint in endpoints {
            self.post(&endpoint, &body).await;
        }
        Ok(())
    }

    async fn post(&self, endpoint: &repository::Endpoint, body: &[u8]) {
        let Some(signature) = sign(&endpoint.secret, body) else {
            tracing::error!(url = %endpoint.url, "webhook body could not be signed");
            self.metrics.webhook_failed();
            return;
        };
        match self
            .http
            .post(&endpoint.url)
            .header(SIGNATURE, signature)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec())
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => {
                tracing::error!(
                    url = %endpoint.url,
                    status = %response.status(),
                    "webhook delivery failed"
                );
                self.metrics.webhook_failed();
            }
            Err(err) => {
                tracing::error!(url = %endpoint.url, error = %err, "webhook delivery failed");
                self.metrics.webhook_failed();
            }
        }
    }
}

fn require_url(url: &str) -> Result<String, AppError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(AppError::WebhookUrlRequired);
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(AppError::WebhookUrlInvalid);
    }
    Ok(url.to_owned())
}

fn sign(secret: &str, body: &[u8]) -> Option<String> {
    let mut mac = Signer::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    Some(hex_encode(&digest))
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_must_be_present_and_absolute() {
        assert!(matches!(
            require_url("  "),
            Err(AppError::WebhookUrlRequired)
        ));
        assert!(matches!(
            require_url("example.com/hook"),
            Err(AppError::WebhookUrlInvalid)
        ));
        assert_eq!(
            require_url("  https://example.com/hook  ").unwrap(),
            "https://example.com/hook"
        );
        assert_eq!(
            require_url("http://example.com/hook").unwrap(),
            "http://example.com/hook"
        );
    }

    #[test]
    fn signature_is_hex_hmac_sha256() {
        let signature = sign("secret", b"body").unwrap();
        assert_eq!(
            signature,
            "dc46983557fea127b43af721467eb9b3fde2338fe3e14f51952aa8478c13d355"
        );
        assert_ne!(sign("other", b"body").unwrap(), signature);
    }
}
