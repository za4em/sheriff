use std::time::Duration;

use prometheus::core::Collector;
use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};

pub(crate) mod http;

const BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Process metrics. Prometheus scrapes [`Self::render`].
#[derive(Clone)]
pub(crate) struct Metrics {
    registry: Registry,
    http_duration: HistogramVec,
    decisions: IntCounterVec,
    provider_duration: Histogram,
    queue_depth: IntGauge,
    retries: IntCounter,
    webhook_failures: IntCounter,
    websockets: IntGauge,
}

impl Metrics {
    pub(crate) fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();
        let http_duration = HistogramVec::new(
            HistogramOpts::new(
                "sheriff_http_request_duration_seconds",
                "HTTP request duration.",
            )
            .buckets(BUCKETS.to_vec()),
            &["method", "path", "status"],
        )?;
        let decisions = IntCounterVec::new(
            Opts::new(
                "sheriff_decisions_total",
                "Decisions that finished, by outcome.",
            ),
            &["outcome"],
        )?;
        let provider_duration = Histogram::with_opts(
            HistogramOpts::new(
                "sheriff_provider_duration_seconds",
                "Provider call duration.",
            )
            .buckets(BUCKETS.to_vec()),
        )?;
        let queue_depth = IntGauge::new(
            "sheriff_score_queue_depth",
            "Messages waiting on the score queue, not counting jobs a worker already took.",
        )?;
        let retries = IntCounter::new(
            "sheriff_score_retries_total",
            "Score jobs put back on the queue after a failure.",
        )?;
        let webhook_failures = IntCounter::new(
            "sheriff_webhook_failures_total",
            "Webhook deliveries that failed.",
        )?;
        let websockets = IntGauge::new(
            "sheriff_websocket_connections",
            "Open websocket connections.",
        )?;
        for metric in [
            Box::new(http_duration.clone()) as Box<dyn Collector>,
            Box::new(decisions.clone()),
            Box::new(provider_duration.clone()),
            Box::new(queue_depth.clone()),
            Box::new(retries.clone()),
            Box::new(webhook_failures.clone()),
            Box::new(websockets.clone()),
        ] {
            registry.register(metric)?;
        }
        Ok(Self {
            registry,
            http_duration,
            decisions,
            provider_duration,
            queue_depth,
            retries,
            webhook_failures,
            websockets,
        })
    }

    pub(crate) fn render(&self) -> Result<String, prometheus::Error> {
        TextEncoder::new().encode_to_string(&self.registry.gather())
    }

    pub(crate) fn observe_http(&self, method: &str, path: &str, status: u16, elapsed: Duration) {
        self.http_duration
            .with_label_values(&[method, path, &status.to_string()])
            .observe(elapsed.as_secs_f64());
    }

    /// Creates the series at zero so a quiet outcome still appears.
    pub(crate) fn prepare_decision(&self, outcome: &str) {
        let _ = self.decisions.with_label_values(&[outcome]);
    }

    pub(crate) fn decision(&self, outcome: &str) {
        self.decisions.with_label_values(&[outcome]).inc();
    }

    pub(crate) fn observe_provider(&self, elapsed: Duration) {
        self.provider_duration.observe(elapsed.as_secs_f64());
    }

    pub(crate) fn set_queue_depth(&self, depth: u32) {
        self.queue_depth.set(i64::from(depth));
    }

    pub(crate) fn retried(&self) {
        self.retries.inc();
    }

    pub(crate) fn webhook_failed(&self) {
        self.webhook_failures.inc();
    }

    pub(crate) fn open_websocket(&self) {
        self.websockets.inc();
    }

    pub(crate) fn close_websocket(&self) {
        self.websockets.dec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposition_lists_the_recorded_samples() {
        let metrics = Metrics::new().unwrap();
        metrics.prepare_decision("allow");
        metrics.prepare_decision("failed");
        metrics.decision("allow");
        metrics.retried();
        metrics.webhook_failed();
        metrics.set_queue_depth(4);
        metrics.open_websocket();
        metrics.observe_provider(Duration::from_millis(40));
        metrics.observe_http("POST", "/v1/score", 200, Duration::from_millis(12));

        let body = metrics.render().unwrap();
        assert!(body.contains("sheriff_decisions_total{outcome=\"allow\"} 1\n"));
        assert!(body.contains("sheriff_decisions_total{outcome=\"failed\"} 0\n"));
        assert!(body.contains("sheriff_score_retries_total 1\n"));
        assert!(body.contains("sheriff_webhook_failures_total 1\n"));
        assert!(body.contains("sheriff_score_queue_depth 4\n"));
        assert!(body.contains("sheriff_websocket_connections 1\n"));
        assert!(body.contains("sheriff_provider_duration_seconds_count 1\n"));
        assert!(body.contains(
            "sheriff_http_request_duration_seconds_count{method=\"POST\",path=\"/v1/score\",status=\"200\"} 1\n"
        ));

        metrics.close_websocket();
        let body = metrics.render().unwrap();
        assert!(body.contains("sheriff_websocket_connections 0\n"));
    }
}
