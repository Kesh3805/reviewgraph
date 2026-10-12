//! Shared builders for gateway tests.
#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use model_gateway::{
    CallBudget, InputSection, ModelRequest, ModelTier, OutputSchema, StructuredInput, SystemPrompt,
    TaskType, TenantScope,
};
use review_core::ids::{OrganizationId, RepositoryId};
use serde_json::json;

pub fn tenant() -> TenantScope {
    TenantScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

pub fn input() -> StructuredInput {
    StructuredInput::new(
        SystemPrompt {
            prompt_id: "correctness".into(),
            prompt_version: "v1".into(),
            prompt_sha: "sha-1".into(),
            text: Arc::from("You are a reviewer."),
        },
        vec![
            InputSection::new("rules", json!({"a": 1})).with_cache_breakpoint(),
            InputSection::new("diff", json!({"files": ["a.ts", "b.ts"]})),
        ],
    )
}

pub fn schema() -> OutputSchema {
    OutputSchema::new(
        "result",
        "1",
        json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false}),
    )
}

/// Gateway metrics backed by an in-memory OpenTelemetry exporter.
pub struct TestMetrics {
    pub provider: opentelemetry_sdk::metrics::SdkMeterProvider,
    pub exporter: opentelemetry_sdk::metrics::InMemoryMetricExporter,
    pub metrics: model_gateway::GatewayMetrics,
}

impl TestMetrics {
    pub fn new() -> Self {
        use opentelemetry::metrics::MeterProvider as _;
        let exporter = opentelemetry_sdk::metrics::InMemoryMetricExporter::default();
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter.clone()).build();
        let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_reader(reader)
            .build();
        let metrics = model_gateway::GatewayMetrics::new(&provider.meter("test"));
        Self {
            provider,
            exporter,
            metrics,
        }
    }

    /// Flushes and returns, per metric name, the sum of the u64 sum points (or the histogram
    /// count) and the number of data points.
    pub fn snapshot(&self) -> std::collections::BTreeMap<String, (u64, usize)> {
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        self.provider.force_flush().expect("flush");
        let mut out = std::collections::BTreeMap::new();
        let all = self.exporter.get_finished_metrics().expect("metrics");
        if let Some(rm) = all.last() {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    let entry: (u64, usize) = match m.data() {
                        AggregatedMetrics::U64(MetricData::Sum(s)) => {
                            let points: Vec<u64> = s.data_points().map(|p| p.value()).collect();
                            (points.iter().sum(), points.len())
                        }
                        AggregatedMetrics::F64(MetricData::Histogram(h)) => (
                            h.data_points().map(|p| p.count()).sum(),
                            h.data_points().count(),
                        ),
                        _ => (0, 0),
                    };
                    out.insert(m.name().to_owned(), entry);
                }
            }
        }
        out
    }

    pub fn sum(&self, name: &str) -> u64 {
        self.snapshot().get(name).map_or(0, |e| e.0)
    }
}

impl Default for TestMetrics {
    fn default() -> Self {
        Self::new()
    }
}

pub fn request() -> ModelRequest {
    ModelRequest::new(
        TaskType::CorrectnessReview,
        ModelTier::ReviewReasoner,
        input(),
        tenant(),
        CallBudget::within(Duration::from_secs(60)),
    )
    .with_schema(schema())
}
