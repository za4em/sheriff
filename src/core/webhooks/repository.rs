use sqlx::PgPool;

use crate::error::{AppError, database};

#[derive(sqlx::FromRow)]
pub(super) struct Endpoint {
    pub(super) url: String,
    pub(super) secret: String,
}

pub(super) async fn insert(
    pool: &PgPool,
    tenant: &str,
    url: &str,
    secret: &str,
) -> Result<i64, AppError> {
    sqlx::query_scalar(
        r#"
        INSERT INTO webhook_endpoints (tenant_id, url, secret)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(tenant)
    .bind(url)
    .bind(secret)
    .fetch_one(pool)
    .await
    .map_err(|err| database("registering a webhook", err))
}

pub(super) async fn endpoints(pool: &PgPool, tenant: &str) -> Result<Vec<Endpoint>, AppError> {
    sqlx::query_as(
        r#"
        SELECT url, secret
        FROM webhook_endpoints
        WHERE tenant_id = $1
        "#,
    )
    .bind(tenant)
    .fetch_all(pool)
    .await
    .map_err(|err| database("loading webhook endpoints", err))
}
