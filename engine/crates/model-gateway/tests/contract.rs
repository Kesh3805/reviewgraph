#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use model_gateway::testing::FakeGateway;
use model_gateway::{
    request_hash, BudgetKind, CallBudget, FinishReason, GatewayBuilder, GatewayError, ModelGateway,
    ModelOutput, ModelTier, ProviderAdapter, ProviderId, ProviderRequest, ProviderResponse,
    RouteCandidate, RouteDecision, StaticRouter, TaskType, Usage,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use common::{input, request};

#[test]
fn request_hash_is_stable_across_runs() {
    let h = request_hash(&request());
    // Golden value: changing the hash recipe invalidates every replay fixture.
    assert_eq!(
        h.as_str(),
        "f3089a82ce4c07ed87bcdd3e8b8c3ef8b75ba19fcebdabab43ae3204a4f73de2"
    );
}

#[test]
fn request_hash_ignores_provider_and_model() {
    // Provider and model are not part of the request at all; route choice cannot move the hash.
    let a = request();
    let mut b = request();
    b.tier = ModelTier::ReviewReasoner;
    b.trace.request_id = Some("x".into());
    b.idempotency_hint = Some("y".into());
    assert_eq!(request_hash(&a), request_hash(&b));
}

#[test]
fn request_hash_changes_with_prompt_version() {
    let a = request();
    let mut b = request();
    b.input.system.prompt_version = "v2".into();
    assert_ne!(request_hash(&a), request_hash(&b));
}

#[test]
fn request_hash_changes_with_schema_hash() {
    let a = request();
    let mut b = request();
    b.output_schema = Some(model_gateway::OutputSchema::new(
        "result",
        "1",
        json!({"type": "object"}),
    ));
    assert_ne!(request_hash(&a), request_hash(&b));
}

#[test]
fn request_hash_section_order_matters() {
    let a = request();
    let mut b = request();
    b.input.sections.reverse();
    assert_ne!(request_hash(&a), request_hash(&b));
}

#[test]
fn debug_never_prints_section_content() {
    let mut i = input();
    i.sections[0].content = json!({"secret": "CANARY-VALUE"});
    let dbg = format!("{i:?}");
    assert!(!dbg.contains("CANARY-VALUE"), "{dbg}");
    assert!(!dbg.contains("You are a reviewer"), "{dbg}");
    assert!(dbg.contains("rules"));
}

struct CountingAdapter(Arc<AtomicUsize>);

#[async_trait]
impl ProviderAdapter for CountingAdapter {
    fn provider(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    async fn send(&self, _req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ProviderResponse {
            output: ModelOutput::Json(json!({"ok": true})),
            usage: Usage::default(),
            finish_reason: FinishReason::Complete,
            model: "m".into(),
            provider_request_id: None,
            tool_use_id: None,
            served_from: model_gateway::ServedFrom::Live,
        })
    }
}

fn gateway(counter: Arc<AtomicUsize>) -> model_gateway::Gateway {
    let router = StaticRouter::new().with_route(
        ModelTier::ReviewReasoner,
        vec![RouteCandidate {
            provider: ProviderId::new("anthropic"),
            model: "m".into(),
            max_context: 200_000,
            supports_reasoning: false,
        }],
    );
    GatewayBuilder::new()
        .adapter(Arc::new(CountingAdapter(counter)))
        .router(Arc::new(router))
        .build()
        .expect("build")
}

#[tokio::test]
async fn expired_deadline_fails_without_io() {
    let counter = Arc::new(AtomicUsize::new(0));
    let gw = gateway(counter.clone());
    let mut req = request();
    req.budget = CallBudget::within(Duration::from_secs(0));
    let err = gw
        .call(req, CancellationToken::new())
        .await
        .expect_err("must fail");
    assert!(matches!(
        err,
        GatewayError::BudgetExceeded {
            kind: BudgetKind::Deadline
        }
    ));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gateway_routes_and_returns_response() {
    let counter = Arc::new(AtomicUsize::new(0));
    let gw = gateway(counter.clone());
    let resp = gw
        .call(request(), CancellationToken::new())
        .await
        .expect("ok");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    assert_eq!(resp.provider.as_str(), "anthropic");
    assert_eq!(resp.request_hash, request_hash(&request()));
    let _: &RouteDecision = &resp.route;
    let _: HashSet<u8> = HashSet::new();
}

#[tokio::test]
async fn cancel_token_aborts_call() {
    let fake = FakeGateway::new().on_pending(TaskType::CorrectnessReview);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        c2.cancel();
    });
    let err = fake.call(request(), cancel).await.expect_err("cancelled");
    assert!(matches!(err, GatewayError::Cancelled));
}

#[tokio::test]
async fn fake_gateway_serves_scripted_json() {
    let fake = FakeGateway::new().on_json(TaskType::CorrectnessReview, json!({"ok": true}));
    let r = fake
        .call(request(), CancellationToken::new())
        .await
        .expect("ok");
    assert_eq!(r.output, ModelOutput::Json(json!({"ok": true})));
    assert_eq!(fake.calls().len(), 1);
}
