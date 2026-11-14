mod messages;
mod rooms;
mod tenant;
mod webhooks;

use crate::app::App;

pub(crate) fn router(app: App) -> axum::Router {
    axum::Router::new()
        .merge(messages::routes())
        .merge(rooms::routes())
        .merge(webhooks::routes())
        .with_state(app)
}
