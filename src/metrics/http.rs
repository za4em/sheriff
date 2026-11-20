use std::{convert::Infallible, time::Instant};

use axum::{
    extract::{MatchedPath, Request, State},
    http::{StatusCode, header::CONTENT_TYPE},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{MethodRouter, get},
};

use super::Metrics;

const METRICS_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

pub(crate) fn endpoint(metrics: Metrics) -> MethodRouter<(), Infallible> {
    get(serve).with_state(metrics)
}

pub(crate) async fn record(
    State(metrics): State<Metrics>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().as_str().to_owned();
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("unmatched")
        .to_owned();
    let started = Instant::now();
    let response = next.run(request).await;
    metrics.observe_http(
        &method,
        &path,
        response.status().as_u16(),
        started.elapsed(),
    );
    response
}

async fn serve(State(metrics): State<Metrics>) -> Response {
    match metrics.render() {
        Ok(body) => ([(CONTENT_TYPE, METRICS_TYPE)], body).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "metrics could not be encoded");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
