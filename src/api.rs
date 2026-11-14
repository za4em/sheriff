mod messages;
mod tenant;

use crate::app::App;

pub(crate) fn router(app: App) -> axum::Router {
    axum::Router::new()
        .merge(messages::routes())
        .with_state(app)
}
