use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use super::tenant::Tenant;
use crate::{app::App, core::messages::IncomingMessage, error::AppError};

pub(crate) fn routes() -> Router<App> {
    Router::new().route("/v1/messages", post(post_messages))
}

async fn post_messages(
    tenant: Tenant,
    State(app): State<App>,
    Json(messages): Json<Vec<IncomingMessage>>,
) -> Result<StatusCode, AppError> {
    app.engine.submit(&tenant.id, messages, None).await?;
    Ok(StatusCode::ACCEPTED)
}
