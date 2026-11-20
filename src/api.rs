mod messages;
mod rooms;
mod score;
mod tenant;
mod webhooks;
mod ws;

use axum::middleware;

use crate::{app::App, metrics};

pub(crate) fn router(app: App) -> axum::Router {
    axum::Router::new()
        .merge(messages::routes())
        .merge(score::routes())
        .merge(rooms::routes())
        .merge(webhooks::routes())
        .merge(ws::routes())
        .route_layer(middleware::from_fn_with_state(
            app.metrics.clone(),
            metrics::http::record,
        ))
        .route_service("/metrics", metrics::http::endpoint(app.metrics.clone()))
        .with_state(app)
}
