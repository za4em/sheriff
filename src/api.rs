mod messages;
mod rooms;
mod tenant;

use crate::app::App;

pub(crate) fn router(app: App) -> axum::Router {
    axum::Router::new()
        .merge(messages::routes())
        .merge(rooms::routes())
        .with_state(app)
}
