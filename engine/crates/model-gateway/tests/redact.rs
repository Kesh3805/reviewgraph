#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! GW-010: the mandatory pre-send redaction pass.
//!
//! `builder_requires_redactor` is a compile-fail doctest on `GatewayBuilder`: without a redactor
//! the builder has no `build()`.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use model_gateway::adapters::anthropic::AnthropicAdapter;
use model_gateway::{
    request_hash, DefaultRedactor, FinishReason, GatewayBuilder, GatewayError, InputSection,
    MemoryLedger, ModelGateway, ModelOutput, ModelTier, PermanentKind, PreSendRedactor,
    ProviderAdapter, ProviderId, ProviderRequest, ProviderResponse, RedactionReport,
    RouteCandidate, ServedFrom, StaticRouter, StructuredInput, Usage,
};
use serde_json::json;
use telemetry::Secret;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{input, request, TestMetrics};

const GH_CANARY: &str = "ghp_CANARYabcdefghijklmnopqrstuvwxyz0123";
const PEM_CANARY: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAcanaryKEYmaterial\n-----END RSA PRIVATE KEY-----";

fn with_section(content: serde_json::Value) -> StructuredInput {
    let mut i = input();
    i.sections.push(InputSection::new("code", content));
    i
}

fn section_text(i: &StructuredInput, name: &str) -> String {
    i.sections
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.content.to_string())
        .unwrap_or_default()
}

#[test]
fn redacts_pem_private_key() {
    let mut i = with_section(json!({"body_head": format!("const k = `{PEM_CANARY}`;")}));
    let report = DefaultRedactor::new().redact(&mut i);
    let text = section_text(&i, "code");
    assert!(!text.contains("canaryKEYmaterial"), "{text}");
    assert!(text.contains("«redacted:pem_private_key:"), "{text}");
    assert_eq!(report.by_pattern.get("pem_private_key"), Some(&1));
    // Not a cache-breakpoint section: redacted, not blocked.
    assert!(!report.blocked);
}

#[test]
fn pem_in_cached_system_context_blocks() {
    let mut i = input();
    i.sections[0].content = json!({"rule": PEM_CANARY});
    assert!(i.sections[0].cache_breakpoint);
    let report = DefaultRedactor::new().redact(&mut i);
    assert!(report.blocked);
}

#[test]
fn redacts_github_token() {
    let mut i = with_section(json!({"excerpt": [format!("const t = '{GH_CANARY}';")]}));
    let report = DefaultRedactor::new().redact(&mut i);
    let text = section_text(&i, "code");
    assert!(!text.contains("CANARYabc"), "{text}");
    assert_eq!(report.replacements, 1);
    assert_eq!(report.by_pattern.get("github_token"), Some(&1));
}

#[test]
fn redacts_env_assignment() {
    let mut i = with_section(
        json!({"file": ".env.example", "body": "PORT=3000\nSTRIPE_SECRET_KEY=sk_live_canary_value\n"}),
    );
    DefaultRedactor::new().redact(&mut i);
    let text = section_text(&i, "code");
    assert!(!text.contains("sk_live_canary_value"), "{text}");
    assert!(text.contains("PORT=3000"), "{text}");
}

#[test]
fn redacts_known_secret_fingerprints() {
    let mut i = with_section(json!({"body": "const conn = 'opaque-internal-credential-42';"}));
    let r = DefaultRedactor::new().with_known_secrets(["opaque-internal-credential-42".to_owned()]);
    let report = r.redact(&mut i);
    assert!(!section_text(&i, "code").contains("opaque-internal"));
    assert_eq!(report.by_pattern.get("known_secret"), Some(&1));
}

#[test]
fn same_secret_same_placeholder() {
    let mut a = with_section(json!({"x": GH_CANARY}));
    let mut b = with_section(json!({"y": [GH_CANARY]}));
    DefaultRedactor::new().redact(&mut a);
    DefaultRedactor::new().redact(&mut b);
    let pa = a.sections[2].content["x"].as_str().unwrap().to_owned();
    let pb = b.sections[2].content["y"][0].as_str().unwrap().to_owned();
    assert_eq!(pa, pb);
    // Redaction is deterministic, so the request hash is stable.
    let mut r1 = request();
    r1.input = with_section(json!({"x": GH_CANARY}));
    let mut r2 = r1.clone();
    DefaultRedactor::new().redact(&mut r1.input);
    DefaultRedactor::new().redact(&mut r2.input);
    assert_eq!(request_hash(&r1), request_hash(&r2));
}

struct Echo {
    calls: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ProviderAdapter for Echo {
    fn provider(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.bodies
            .lock()
            .unwrap()
            .push(serde_json::to_string(&req.request.input).unwrap());
        Ok(ProviderResponse {
            output: ModelOutput::Json(json!({"ok": true})),
            usage: Usage::default(),
            finish_reason: FinishReason::Complete,
            model: "m".into(),
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

fn gateway_with(
    redactor: Arc<dyn PreSendRedactor>,
    adapter: Arc<dyn ProviderAdapter>,
    ledger: Arc<MemoryLedger>,
    metrics: &TestMetrics,
) -> model_gateway::Gateway {
    GatewayBuilder::new()
        .redactor(redactor)
        .adapter(adapter)
        .router(Arc::new(
            StaticRouter::new().with_route(ModelTier::ReviewReasoner, vec![candidate()]),
        ))
        .ledger(ledger)
        .metrics(metrics.metrics.clone())
        .build()
        .expect("build")
}

#[tokio::test]
async fn request_hash_computed_after_redaction() {
    let calls = Arc::new(AtomicUsize::new(0));
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let ledger = Arc::new(MemoryLedger::new());
    let metrics = TestMetrics::new();
    let gw = gateway_with(
        Arc::new(DefaultRedactor::new()),
        Arc::new(Echo {
            calls: calls.clone(),
            bodies: bodies.clone(),
        }),
        ledger.clone(),
        &metrics,
    );
    let mut req = request();
    req.input = with_section(json!({"body": GH_CANARY}));
    let original = request_hash(&req);
    let mut redacted = req.clone();
    DefaultRedactor::new().redact(&mut redacted.input);

    let resp = gw
        .call(req, CancellationToken::new())
        .await
        .expect("served");
    assert_eq!(resp.request_hash, request_hash(&redacted));
    assert_ne!(resp.request_hash, original);
    assert_eq!(ledger.records()[0].request_hash, resp.request_hash.0);
    assert!(!bodies.lock().unwrap()[0].contains("CANARYabc"));
    assert_eq!(metrics.sum("llm_redactions_total"), 1);
}

#[derive(Debug)]
struct Panics;

impl PreSendRedactor for Panics {
    fn redact(&self, _input: &mut StructuredInput) -> RedactionReport {
        panic!("redactor bug");
    }
}

#[tokio::test]
async fn redactor_panic_blocks_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let metrics = TestMetrics::new();
    let gw = gateway_with(
        Arc::new(Panics),
        Arc::new(Echo {
            calls: calls.clone(),
            bodies: Arc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(MemoryLedger::new()),
        &metrics,
    );
    let err = gw
        .call(request(), CancellationToken::new())
        .await
        .expect_err("blocked");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::InvalidRequest,
            ..
        }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(metrics.sum("llm_blocked_requests_total"), 1);
}

#[tokio::test]
async fn secret_in_system_prompt_blocks_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let metrics = TestMetrics::new();
    let gw = gateway_with(
        Arc::new(DefaultRedactor::new()),
        Arc::new(Echo {
            calls: calls.clone(),
            bodies: Arc::new(Mutex::new(Vec::new())),
        }),
        Arc::new(MemoryLedger::new()),
        &metrics,
    );
    let mut req = request();
    req.input.system.text = Arc::from(format!("You are a reviewer. {PEM_CANARY}"));
    let err = gw
        .call(req, CancellationToken::new())
        .await
        .expect_err("blocked");
    assert!(matches!(err, GatewayError::Permanent { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Acceptance: a request whose sections carry `ghp_` and PEM canaries reaches the provider with
/// neither canary in the HTTP body.
#[tokio::test]
async fn canaries_never_reach_provider_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-5-5",
            "content": [{"type": "tool_use", "id": "toolu_1", "name": "emit_result", "input": {"ok": true}}],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 2}
        })))
        .mount(&server)
        .await;
    let adapter =
        AnthropicAdapter::new(Secret::new("sk-ant-test-key"), Some(server.uri())).expect("adapter");
    let metrics = TestMetrics::new();
    let gw = gateway_with(
        Arc::new(DefaultRedactor::new()),
        Arc::new(adapter),
        Arc::new(MemoryLedger::new()),
        &metrics,
    );
    let mut req = request();
    req.input = with_section(json!({"token": GH_CANARY, "key": PEM_CANARY}));
    req.budget = model_gateway::CallBudget::within(Duration::from_secs(30));
    gw.call(req, CancellationToken::new())
        .await
        .expect("served");
    let received = server.received_requests().await.expect("recorded");
    assert_eq!(received.len(), 1);
    let body = String::from_utf8_lossy(&received[0].body).to_string();
    assert!(!body.contains("CANARYabc"), "{body}");
    assert!(!body.contains("canaryKEYmaterial"), "{body}");
    assert!(body.contains("redacted:github_token"), "{body}");
}
