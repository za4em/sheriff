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
    UnknownCategory,
    WebhookUrlRequired,
    WebhookUrlInvalid,
    WebhookEncode,
    InvalidLimit,
    Database { action: &'static str },
    Queue { action: &'static str },
    Provider(Fault),
}

pub(crate) fn database(action: &'static str, err: impl Display) -> AppError {
    tracing::error!(error = %err, "database failed while {action}");
    AppError::Database { action }
}

pub(crate) fn queue(action: &'static str, err: impl Display) -> AppError {
    tracing::error!(error = %err, "queue failed while {action}");
    AppError::Queue { action }
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
        | AppError::UnknownCategory
        | AppError::WebhookUrlRequired
        | AppError::WebhookUrlInvalid
        | AppError::InvalidLimit => StatusCode::BAD_REQUEST,
        AppError::WebhookEncode | AppError::Database { .. } | AppError::Queue { .. } => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
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
        AppError::UnknownCategory => {
            "A category must be insult, bullying, grooming, hate, threat, scam, illegal_activity, doxxing, or spam.".to_owned()
        }
        AppError::WebhookUrlRequired => "A webhook URL is required.".to_owned(),
        AppError::WebhookUrlInvalid => {
            "A webhook URL must start with http:// or https://.".to_owned()
        }
        AppError::WebhookEncode => "A webhook body could not be encoded.".to_owned(),
        AppError::InvalidLimit => "A limit must be from 1 to 100.".to_owned(),
        AppError::Database { action } => format!("The database failed while {action}."),
        AppError::Queue { action } => format!("The queue failed while {action}."),
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
