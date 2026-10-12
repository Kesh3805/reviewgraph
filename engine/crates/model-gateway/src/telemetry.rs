//! Gateway metric instruments (GW-010): the central registry for every GW metric.
//!
//! Instruments are created once per gateway from a [`Meter`] (the global meter by default) and are
//! lock-free. Labels carry provider, model, task and tier names only; prompt and output text never
//! reach a metric or a span.

use opentelemetry::metrics::{Counter, Histogram, Meter};
use opentelemetry::KeyValue;

/// Histogram buckets of `llm_request_duration_seconds`.
pub const DURATION_BUCKETS: [f64; 11] =
    [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 15.0, 30.0, 60.0, 90.0, 120.0];

/// Every metric instrument the gateway emits.
#[derive(Clone)]
pub struct GatewayMetrics {
    requests: Counter<u64>,
    duration: Histogram<f64>,
    input_tokens: Counter<u64>,
    output_tokens: Counter<u64>,
    cached_tokens: Counter<u64>,
    cost: Counter<u64>,
    retries: Counter<u64>,
    rate_limited: Counter<u64>,
    fallbacks: Counter<u64>,
    schema_repairs: Counter<u64>,
    structured_failures: Counter<u64>,
    structured_success: Counter<u64>,
    redactions: Counter<u64>,
    blocked: Counter<u64>,
    cache_hits: Counter<u64>,
    cache_misses: Counter<u64>,
}

impl std::fmt::Debug for GatewayMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayMetrics").finish_non_exhaustive()
    }
}

fn counter(meter: &Meter, name: &'static str, description: &'static str) -> Counter<u64> {
    meter
        .u64_counter(name)
        .with_description(description)
        .build()
}

impl GatewayMetrics {
    /// Registers the instruments on `meter`.
    pub fn new(meter: &Meter) -> Self {
        Self {
            requests: counter(
                meter,
                "llm_requests_total",
                "Logical model calls by outcome",
            ),
            duration: meter
                .f64_histogram("llm_request_duration_seconds")
                .with_description("Wall time of a logical model call")
                .with_unit("s")
                .with_boundaries(DURATION_BUCKETS.to_vec())
                .build(),
            input_tokens: counter(meter, "llm_input_tokens_total", "Uncached input tokens"),
            output_tokens: counter(meter, "llm_output_tokens_total", "Output tokens"),
            cached_tokens: counter(
                meter,
                "llm_cached_tokens_total",
                "Prompt-cache tokens by kind (read, write)",
            ),
            cost: counter(
                meter,
                "llm_cost_estimate",
                "Estimated cost in micro-USD from the price table",
            ),
            retries: counter(
                meter,
                "llm_retries_total",
                "Provider attempts after the first",
            ),
            rate_limited: counter(meter, "llm_rate_limited_total", "Rate-limited attempts"),
            fallbacks: counter(
                meter,
                "llm_fallbacks_total",
                "Fallbacks to the next candidate",
            ),
            schema_repairs: counter(
                meter,
                "llm_schema_repairs_total",
                "Repair turns sent after invalid structured output",
            ),
            structured_failures: counter(
                meter,
                "structured_output_failures_total",
                "Invalid structured outputs by stage (first, after_repair)",
            ),
            structured_success: counter(
                meter,
                "structured_output_success_total",
                "Calls whose structured output validated",
            ),
            redactions: counter(
                meter,
                "llm_redactions_total",
                "Secrets replaced before send, by pattern",
            ),
            blocked: counter(
                meter,
                "llm_blocked_requests_total",
                "Calls refused before send, by reason",
            ),
            cache_hits: counter(meter, "model_cache_hits_total", "Response-cache hits"),
            cache_misses: counter(meter, "model_cache_misses_total", "Response-cache misses"),
        }
    }

    /// Instruments on the global meter provider (installed by `telemetry::init`).
    pub fn global() -> Self {
        Self::new(&opentelemetry::global::meter("model-gateway"))
    }

    pub(crate) fn request(&self, labels: &CallLabels, outcome: &'static str, seconds: f64) {
        let mut attrs = labels.full();
        attrs.push(KeyValue::new("outcome", outcome));
        self.requests.add(1, &attrs);
        self.duration.record(seconds, &labels.short());
    }

    pub(crate) fn usage(
        &self,
        labels: &CallLabels,
        usage: &crate::types::Usage,
        cost: Option<u64>,
    ) {
        let attrs = labels.short();
        self.input_tokens
            .add(u64::from(usage.input_uncached), &attrs);
        self.output_tokens.add(u64::from(usage.output), &attrs);
        if usage.cache_read > 0 {
            let mut a = attrs.clone();
            a.push(KeyValue::new("kind", "read"));
            self.cached_tokens.add(u64::from(usage.cache_read), &a);
        }
        if usage.cache_write > 0 {
            let mut a = attrs.clone();
            a.push(KeyValue::new("kind", "write"));
            self.cached_tokens.add(u64::from(usage.cache_write), &a);
        }
        if let Some(c) = cost {
            self.cost.add(c, &attrs);
        }
    }

    pub(crate) fn retries(&self, labels: &CallLabels, n: u64) {
        if n > 0 {
            self.retries.add(n, &labels.short());
        }
    }

    pub(crate) fn rate_limited(&self, provider: &str) {
        self.rate_limited
            .add(1, &[KeyValue::new("provider", provider.to_owned())]);
    }

    pub(crate) fn fallback(&self, labels: &CallLabels, reason: &'static str) {
        let mut attrs = labels.short();
        attrs.push(KeyValue::new("reason", reason));
        self.fallbacks.add(1, &attrs);
    }

    pub(crate) fn schema_repair(&self, task: &'static str, provider: &str) {
        self.schema_repairs.add(1, &task_provider(task, provider));
    }

    pub(crate) fn structured_failure(
        &self,
        task: &'static str,
        provider: &str,
        stage: &'static str,
    ) {
        let mut attrs = task_provider(task, provider);
        attrs.push(KeyValue::new("stage", stage));
        self.structured_failures.add(1, &attrs);
    }

    pub(crate) fn structured_success(&self, task: &'static str, provider: &str) {
        self.structured_success
            .add(1, &task_provider(task, provider));
    }

    pub(crate) fn redactions(&self, by_pattern: &std::collections::BTreeMap<&'static str, u32>) {
        for (&pattern, &n) in by_pattern {
            self.redactions
                .add(u64::from(n), &[KeyValue::new("pattern", pattern)]);
        }
    }

    pub(crate) fn blocked(&self, reason: &'static str) {
        self.blocked.add(1, &[KeyValue::new("reason", reason)]);
    }

    pub(crate) fn cache_hit(&self, task: &'static str) {
        self.cache_hits.add(1, &[KeyValue::new("task", task)]);
    }

    pub(crate) fn cache_miss(&self, task: &'static str) {
        self.cache_misses.add(1, &[KeyValue::new("task", task)]);
    }
}

fn task_provider(task: &'static str, provider: &str) -> Vec<KeyValue> {
    vec![
        KeyValue::new("task", task),
        KeyValue::new("provider", provider.to_owned()),
    ]
}

/// Label values of one logical call.
#[derive(Debug, Clone)]
pub(crate) struct CallLabels {
    pub provider: String,
    pub model: String,
    pub task: &'static str,
    pub tier: &'static str,
}

impl CallLabels {
    fn short(&self) -> Vec<KeyValue> {
        vec![
            KeyValue::new("provider", self.provider.clone()),
            KeyValue::new("model", self.model.clone()),
            KeyValue::new("task", self.task),
        ]
    }

    fn full(&self) -> Vec<KeyValue> {
        let mut v = self.short();
        v.push(KeyValue::new("tier", self.tier));
        v
    }
}
