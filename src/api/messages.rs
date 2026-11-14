use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use super::tenant::Tenant;
use crate::{
    app::App,
    core::{decisions::Decision, messages::IncomingMessage},
    error::AppError,
};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/v1/messages", post(post_messages))
        .route("/v1/messages/async", post(post_messages_async))
}

async fn post_messages(
    tenant: Tenant,
    State(app): State<App>,
    Json(messages): Json<Vec<IncomingMessage>>,
) -> Result<Json<Vec<Decision>>, AppError> {
    Ok(Json(app.engine.submit_sync(&tenant.id, messages).await?))
}

async fn post_messages_async(
    tenant: Tenant,
    State(app): State<App>,
    Json(messages): Json<Vec<IncomingMessage>>,
) -> Result<StatusCode, AppError> {
    app.engine.submit_async(&tenant.id, messages).await?;
    Ok(StatusCode::ACCEPTED)
}
