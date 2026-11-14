use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;

use super::tenant::Tenant;
use crate::{app::App, core::webhooks::Registered, error::AppError};

pub(crate) fn routes() -> Router<App> {
    Router::new().route("/v1/webhooks", post(create))
}

#[derive(Deserialize)]
struct RegisterRequest {
    url: String,
}

async fn create(
    tenant: Tenant,
    State(app): State<App>,
    Json(request): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<Registered>), AppError> {
    let registered = app
        .engine
        .webhooks
        .register(&tenant.id, &request.url)
        .await?;
    Ok((StatusCode::CREATED, Json(registered)))
}
