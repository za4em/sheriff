mod messages;
mod rooms;
mod score;
mod tenant;
mod webhooks;
mod ws;

use crate::app::App;

pub(crate) fn router(app: App) -> axum::Router {
    axum::Router::new()
        .merge(messages::routes())
        .merge(score::routes())
        .merge(rooms::routes())
        .merge(webhooks::routes())
        .merge(ws::routes())
        .with_state(app)
}
