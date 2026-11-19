use axum::{Json, Router, extract::State, routing::post};

use super::tenant::Tenant;
use crate::{
    app::App,
    core::{decisions::Decision, messages::IncomingMessage},
    error::AppError,
};

pub(crate) fn routes() -> Router<App> {
    Router::new().route("/v1/score", post(score))
}

async fn score(
    Tenant { id }: Tenant,
    State(app): State<App>,
    Json(messages): Json<Vec<IncomingMessage>>,
) -> Result<Json<Vec<Decision>>, AppError> {
    Ok(Json(app.engine.score(&id, messages).await?))
}
