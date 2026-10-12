#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! GW-010: the gateway metric set. The span test lives in `telemetry_spans.rs`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use model_gateway::{
    CachePolicy, CallBudget, DefaultRedactor, FinishReason, GatewayBuilder, GatewayError,
    MemoryCache, ModelGateway, ModelOutput, ModelRequest, ModelTier, ProviderAdapter, ProviderId,
    ProviderRequest, ProviderResponse, RouteCandidate, ServedFrom, StaticRouter, TaskType, Usage,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use common::{input, request, tenant, TestMetrics};

struct Fixed {
    output: serde_json::Value,
    usage: Usage,
}

#[async_trait]
impl ProviderAdapter for Fixed {
    fn provider(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    async fn send(&self, _req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        Ok(ProviderResponse {
            output: ModelOutput::Json(self.output.clone()),
            usage: self.usage,
            finish_reason: FinishReason::Complete,
            model: "claude-sonnet-5-5".into(),
            provider_request_id: None,
            tool_use_id: None,
            served_from: ServedFrom::Live,
        })
    }
}

fn candidate() -> RouteCandidate {
    RouteCandidate {
        provider: ProviderId::new("anthropic"),
        model: "claude-sonnet-5-5".into(),
        max_context: 200_000,
        supports_reasoning: false,
    }
}

fn gateway(output: serde_json::Value, metrics: &TestMetrics) -> model_gateway::Gateway {
    GatewayBuilder::new()
        .redactor(Arc::new(DefaultRedactor::new()))
        .adapter(Arc::new(Fixed {
            output,
            usage: Usage {
                input_uncached: 120,
                cache_read: 30,
                output: 7,
                ..Usage::default()
            },
        }))
        .router(Arc::new(
            StaticRouter::new()
                .with_route(ModelTier::ReviewReasoner, vec![candidate()])
                .with_route(ModelTier::Classifier, vec![candidate()]),
        ))
        .response_cache(Arc::new(MemoryCache::new()))
        .metrics(metrics.metrics.clone())
        .build()
        .expect("build")
}

#[tokio::test]
async fn metrics_emitted_per_call() {
    let metrics = TestMetrics::new();
    let gw = gateway(json!({"ok": true}), &metrics);
    gw.call(request(), CancellationToken::new())
        .await
        .expect("served");
    let snap = metrics.snapshot();
    assert_eq!(snap.get("llm_requests_total").map(|e| e.0), Some(1));
    assert_eq!(
        snap.get("llm_request_duration_seconds").map(|e| e.0),
        Some(1)
    );
    assert_eq!(snap.get("llm_input_tokens_total").map(|e| e.0), Some(120));
    assert_eq!(snap.get("llm_output_tokens_total").map(|e| e.0), Some(7));
    assert_eq!(snap.get("llm_cached_tokens_total").map(|e| e.0), Some(30));
    assert_eq!(
        snap.get("structured_output_success_total").map(|e| e.0),
        Some(1)
    );
}

fn classify_req() -> ModelRequest {
    ModelRequest::new(
        TaskType::IntentClassification,
        ModelTier::Classifier,
        input(),
        tenant(),
        CallBudget::within(Duration::from_secs(60)),
    )
    .with_cache(CachePolicy::PromptAndResponse {
        ttl: Duration::from_secs(600),
    })
}

#[tokio::test]
async fn cache_hit_emits_cache_metric_not_tokens() {
    let metrics = TestMetrics::new();
    let gw = gateway(json!({"label": "bugfix"}), &metrics);
    let req = classify_req();
    let first = gw
        .call(req.clone(), CancellationToken::new())
        .await
        .expect("live");
    assert_eq!(first.served_from, ServedFrom::Live);
    let second = gw
        .call(req, CancellationToken::new())
        .await
        .expect("cached");
    assert_eq!(second.served_from, ServedFrom::ResponseCache);

    let snap = metrics.snapshot();
    assert_eq!(snap.get("model_cache_hits_total").map(|e| e.0), Some(1));
    assert_eq!(snap.get("model_cache_misses_total").map(|e| e.0), Some(1));
    // Only the live call counted tokens.
    assert_eq!(snap.get("llm_input_tokens_total").map(|e| e.0), Some(120));
    assert_eq!(snap.get("llm_requests_total").map(|e| e.0), Some(2));
}
