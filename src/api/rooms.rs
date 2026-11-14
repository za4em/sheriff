use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::Deserialize;

use super::tenant::Tenant;
use crate::{app::App, core::decisions::Decision, error::AppError};

pub(crate) fn routes() -> Router<App> {
    Router::new().route("/v1/rooms/{room_id}/decisions", get(list_decisions))
}

#[derive(Deserialize)]
struct DecisionsQuery {
    #[serde(default)]
    id: Vec<String>,
    user_id: Option<String>,
    after: Option<i64>,
    category: Option<String>,
}

async fn list_decisions(
    tenant: Tenant,
    State(app): State<App>,
    Path(room_id): Path<String>,
    Query(query): Query<DecisionsQuery>,
) -> Result<Json<Vec<Decision>>, AppError> {
    let decisions = app
        .engine
        .decisions
        .list(
            &tenant.id,
            &room_id,
            &query.id,
            query.user_id.as_deref(),
            query.after,
            query.category.as_deref(),
        )
        .await?;
    Ok(Json(decisions))
}
