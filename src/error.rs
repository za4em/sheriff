use std::fmt::{self, Display};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::core::provider::Fault;

#[derive(Debug, PartialEq)]
pub(crate) enum AppError {
    Unauthorized,
    InvalidMessage,
    MessageIncomplete { field: &'static str },
    WebhookUrlRequired,
    WebhookUrlInvalid,
    WebhookEncode,
    Database { action: &'static str },
    Provider(Fault),
}

pub(crate) fn database(action: &'static str, err: impl Display) -> AppError {
    tracing::error!(error = %err, "database failed while {action}");
    AppError::Database { action }
}

impl Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&client_message(self))
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (status(&self), client_message(&self)).into_response()
    }
}

fn status(err: &AppError) -> StatusCode {
    match err {
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::InvalidMessage
        | AppError::MessageIncomplete { .. }
        | AppError::WebhookUrlRequired
        | AppError::WebhookUrlInvalid => StatusCode::BAD_REQUEST,
        AppError::WebhookEncode | AppError::Database { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        AppError::Provider(_) => StatusCode::BAD_GATEWAY,
    }
}

fn client_message(err: &AppError) -> String {
    match err {
        AppError::Unauthorized => {
            "The API key is missing or wrong. Send Authorization: Bearer <key>.".to_owned()
        }
        AppError::InvalidMessage => "Send a JSON array of messages.".to_owned(),
        AppError::MessageIncomplete { field } => {
            format!("A message needs a non-empty {field}.")
        }
        AppError::WebhookUrlRequired => "A webhook URL is required.".to_owned(),
        AppError::WebhookUrlInvalid => {
            "A webhook URL must start with http:// or https://.".to_owned()
        }
        AppError::WebhookEncode => "A webhook body could not be encoded.".to_owned(),
        AppError::Database { action } => format!("The database failed while {action}."),
        AppError::Provider(fault) => {
            format!(
                "Could not decide on the message. {}",
                provider_reason(fault)
            )
        }
    }
}

fn provider_reason(fault: &Fault) -> &'static str {
    match fault {
        Fault::RequestRejected => "The decision service did not accept the request.",
        Fault::BodyUnreadable => "The decision service response could not be read.",
        Fault::Upstream => "The decision service returned an error.",
        Fault::InvalidResponse => "The decision service response was not a set of probabilities.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_response_is_the_status_and_the_sentence() {
        assert_response(
            AppError::Unauthorized,
            StatusCode::UNAUTHORIZED,
            "The API key is missing or wrong. Send Authorization: Bearer <key>.",
        )
        .await;
        assert_response(
            AppError::MessageIncomplete { field: "text" },
            StatusCode::BAD_REQUEST,
            "A message needs a non-empty text.",
        )
        .await;
        assert_response(
            AppError::WebhookEncode,
            StatusCode::INTERNAL_SERVER_ERROR,
            "A webhook body could not be encoded.",
        )
        .await;
        assert_response(
            AppError::Database {
                action: "registering a webhook",
            },
            StatusCode::INTERNAL_SERVER_ERROR,
            "The database failed while registering a webhook.",
        )
        .await;
        assert_response(
            AppError::Provider(Fault::Upstream),
            StatusCode::BAD_GATEWAY,
            "Could not decide on the message. The decision service returned an error.",
        )
        .await;
    }

    async fn assert_response(err: AppError, code: StatusCode, sentence: &str) {
        let response = err.into_response();
        assert_eq!(response.status(), code);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&body).unwrap(), sentence);
    }
}
