#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! GW-010: `model_request` spans and the gateway metric set.

mod common;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use model_gateway::{
    CachePolicy, CallBudget, DefaultRedactor, FinishReason, GatewayBuilder, GatewayError,
    InputSection, MemoryCache, ModelGateway, ModelOutput, ModelRequest, ModelTier, ProviderAdapter,
    ProviderId, ProviderRequest, ProviderResponse, RouteCandidate, ServedFrom, StaticRouter,
    TaskType, Usage,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use common::{input, request, tenant, TestMetrics};

const PROMPT_CANARY: &str = "PROMPT-CANARY-7f3a";
const SECTION_CANARY: &str = "SECTION-CANARY-91c2";
const OUTPUT_CANARY: &str = "OUTPUT-CANARY-55d0";

/// Records every span name, span field and event field as text.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<String>>);

struct Fields<'a>(&'a mut String);

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = write!(self.0, " {}={}", field.name(), value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, " {}={:?}", field.name(), value);
    }
}

impl<S: Subscriber> Layer<S> for Capture {
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        let mut s = format!("\nspan {}", attrs.metadata().name());
        attrs.record(&mut Fields(&mut s));
        self.0.lock().unwrap().push_str(&s);
    }

    fn on_record(&self, _id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut s = String::from("\nrecord");
        values.record(&mut Fields(&mut s));
        self.0.lock().unwrap().push_str(&s);
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut s = String::from("\nevent");
        event.record(&mut Fields(&mut s));
        self.0.lock().unwrap().push_str(&s);
    }
}

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
async fn span_has_no_prompt_or_output_text() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let _guard = tracing::subscriber::set_default(subscriber);

    let metrics = TestMetrics::new();
    // The output carries a canary in a free-text field; the schema allows no such field, so use
    // a request without a schema to let it through unchanged.
    let gw = gateway(json!({"ok": true, "note": OUTPUT_CANARY}), &metrics);
    let mut req = request();
    req.output_schema = None;
    req.input.system.text = Arc::from(format!("You are a reviewer. {PROMPT_CANARY}"));
    req.input
        .sections
        .push(InputSection::new("code", json!({"body": SECTION_CANARY})));
    let resp = gw
        .call(req, CancellationToken::new())
        .await
        .expect("served");
    assert!(matches!(resp.output, ModelOutput::Json(_)));

    let text = capture.0.lock().unwrap().clone();
    assert!(text.contains("span model_request"), "{text}");
    assert!(text.contains("span model_request.attempt"), "{text}");
    assert!(text.contains("rg.request_hash="), "{text}");
    assert!(text.contains("gen_ai.usage.output_tokens=7"), "{text}");
    for canary in [PROMPT_CANARY, SECTION_CANARY, OUTPUT_CANARY] {
        assert!(
            !text.contains(canary),
            "{canary} leaked into telemetry:\n{text}"
        );
    }
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
