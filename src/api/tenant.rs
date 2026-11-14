use axum::extract::FromRequestParts;
use axum::http::{header::AUTHORIZATION, request::Parts};

use crate::{app::App, error::AppError};

/// One configured key, one tenant, until keys are loaded from `tenants`.
pub(crate) struct Tenant {
    pub(crate) id: String,
}

impl FromRequestParts<App> for Tenant {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &App) -> Result<Self, Self::Rejection> {
        let presented = presented_key(parts).ok_or(AppError::Unauthorized)?;
        if !keys_match(presented, &state.api_key) {
            return Err(AppError::Unauthorized);
        }
        Ok(Tenant {
            id: "local".to_owned(),
        })
    }
}

fn presented_key(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(AUTHORIZATION)?.to_str().ok()?.trim();
    value.strip_prefix("Bearer ")
}

fn keys_match(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= *a ^ *b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_compares_every_byte() {
        assert!(keys_match("abc", "abc"));
        assert!(!keys_match("abc", "abd"));
        assert!(!keys_match("abc", "abcd"));
    }
}
