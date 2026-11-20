use std::error::Error;

use sqlx::postgres::PgPoolOptions;

use crate::{
    config::Config,
    core::{Engine, cache::Cache, provider::Provider, queue::Queue},
    metrics::Metrics,
};

#[derive(Clone)]
pub struct App {
    pub(crate) engine: Engine,
    pub(crate) api_key: String,
    pub(crate) metrics: Metrics,
}

impl App {
    pub async fn connect(config: Config) -> Result<Self, Box<dyn Error>> {
        let pool = PgPoolOptions::new()
            .max_connections(config.database.pool_size)
            .connect(&config.database.url)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        let metrics = Metrics::new()?;
        let provider = Provider::from_config(config.provider)?;
        let queue = Queue::connect(&config.queue_url).await?;
        let cache = Cache::connect(&config.cache_url, config.cache_limit).await?;
        let engine = Engine::new(
            pool,
            provider,
            config.window,
            config.threshold,
            queue,
            cache,
            metrics.clone(),
        )?;
        engine.serve(config.workers).await;
        Ok(Self {
            engine,
            api_key: config.api_key,
            metrics,
        })
    }
}
