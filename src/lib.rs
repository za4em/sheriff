mod api;
mod app;
mod config;
mod core;
mod error;
mod metrics;

pub use app::App;
pub use config::Config;

pub fn router(app: App) -> axum::Router {
    api::router(app)
}
