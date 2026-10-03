#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use model_gateway::adapters::anthropic::AnthropicAdapter;
use model_gateway::adapters::anthropic_wire::build_request;
use model_gateway::{
    CachePolicy, FinishReason, GatewayBuilder, GatewayError, InputSection, ModelGateway,
    ModelOutput, ModelTier, PermanentKind, ProviderAdapter, ProviderId, ProviderRequest,
    ReasoningLevel, RetryPolicy, RouteCandidate, StaticRouter, TransientKind,
};
use serde_json::{json, Value};
use telemetry::Secret;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::request;

fn candidate() -> RouteCandidate {
    RouteCandidate {
        provider: ProviderId::new("anthropic"),
        model: "claude-sonnet-5-5".into(),
        max_context: 200_000,
        supports_reasoning: true,
    }
}

fn adapter(server: &MockServer) -> AnthropicAdapter {
    AnthropicAdapter::new(Secret::new("sk-ant-test-key"), Some(server.uri())).expect("adapter")
}

async fn send(
    a: &AnthropicAdapter,
    req: &model_gateway::ModelRequest,
) -> Result<model_gateway::ProviderResponse, GatewayError> {
    let c = candidate();
    a.send(&ProviderRequest {
        request: req,
        candidate: &c,
        timeout: Duration::from_secs(5),
    })
    .await
}

fn tool_use_body() -> Value {
    json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-5-5",
        "content": [{"type": "tool_use", "id": "toolu_1", "name": "emit_result", "input": {"ok": true}}],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 100, "output_tokens": 20, "cache_creation_input_tokens": 300, "cache_read_input_tokens": 4000}
    })
}

fn count_cache_control(v: &Value) -> usize {
    match v {
        Value::Object(m) => {
            usize::from(m.contains_key("cache_control"))
                + m.values().map(count_cache_control).sum::<usize>()
        }
        Value::Array(a) => a.iter().map(count_cache_control).sum(),
        _ => 0,
    }
}

#[test]
fn anthropic_builds_forced_tool_request() {
    let body = build_request(&request(), "claude-sonnet-5-5");
    insta::assert_json_snapshot!(body);
}

#[test]
fn anthropic_places_cache_control_on_system_and_last_breakpoint() {
    let body = build_request(&request(), "m");
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    let content = &body["messages"][0]["content"];
    assert_eq!(content[0]["cache_control"]["type"], "ephemeral");
    assert!(content[1].get("cache_control").is_none());
}

#[test]
fn anthropic_caps_breakpoints_at_four() {
    let mut req = request();
    req.input.sections = (0..8)
        .map(|i| InputSection::new(format!("s{i}"), json!({"i": i})).with_cache_breakpoint())
        .collect();
    let body = build_request(&req, "m");
    assert_eq!(count_cache_control(&body), 4);
}

#[test]
fn anthropic_disabled_cache_sends_no_cache_control() {
    let req = request().with_cache(CachePolicy::Disabled);
    assert_eq!(count_cache_control(&build_request(&req, "m")), 0);
}

#[test]
fn anthropic_schema_forbids_thinking_with_forced_tool() {
    let req = request().with_reasoning(ReasoningLevel::High);
    let body = build_request(&req, "m");
    assert!(body.get("thinking").is_none());
    assert_eq!(body["tool_choice"]["type"], "tool");
}

#[test]
fn anthropic_text_mode_thinking_budget_is_capped() {
    let mut req = request().with_reasoning(ReasoningLevel::High);
    req.output_schema = None;
    req.max_output_tokens = 4_000;
    let body = build_request(&req, "m");
    assert_eq!(body["thinking"]["budget_tokens"], 3_999);
    assert!(body.get("tools").is_none());
}

#[tokio::test]
async fn anthropic_parses_tool_use_output() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-test-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(tool_use_body())
                .insert_header("request-id", "req_abc"),
        )
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.output, ModelOutput::Json(json!({"ok": true})));
    assert_eq!(r.tool_use_id.as_deref(), Some("toolu_1"));
    assert_eq!(r.provider_request_id.as_deref(), Some("req_abc"));
    assert_eq!(r.finish_reason, FinishReason::Complete);
}

#[tokio::test]
async fn anthropic_maps_usage_including_cache_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tool_use_body()))
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.usage.input_uncached, 100);
    assert_eq!(r.usage.cache_write, 300);
    assert_eq!(r.usage.cache_read, 4000);
    assert_eq!(r.usage.output, 20);
}

#[tokio::test]
async fn anthropic_max_tokens_finish_reported() {
    let server = MockServer::start().await;
    let mut body = tool_use_body();
    body["stop_reason"] = json!("max_tokens");
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.finish_reason, FinishReason::MaxTokens);
}

#[tokio::test]
async fn anthropic_refusal_maps_to_refusal() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "m", "content": [{"type": "text", "text": "I cannot help with that"}],
        "stop_reason": "refusal", "usage": {"input_tokens": 1, "output_tokens": 1}
    });
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.finish_reason, FinishReason::Refusal);
}

#[tokio::test]
async fn anthropic_text_only_answer_is_schema_violation() {
    let server = MockServer::start().await;
    let body = json!({
        "model": "m", "content": [{"type": "text", "text": "here you go"}],
        "stop_reason": "end_turn", "usage": {"input_tokens": 1, "output_tokens": 1}
    });
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request())
        .await
        .expect_err("violation");
    assert!(matches!(err, GatewayError::SchemaViolation { .. }));
}

#[tokio::test]
async fn anthropic_429_with_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "12")
                .set_body_json(json!({"type": "error", "error": {"type": "rate_limit_error", "message": "slow down"}})),
        )
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request()).await.expect_err("429");
    match err {
        GatewayError::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(12)))
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn anthropic_529_overloaded_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529).set_body_json(json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tool_use_body()))
        .with_priority(2)
        .mount(&server)
        .await;

    let router = StaticRouter::new().with_route(ModelTier::ReviewReasoner, vec![candidate()]);
    let policy =
        RetryPolicy::default().with_delays(Duration::from_millis(1), Duration::from_millis(5));
    let gw = GatewayBuilder::new()
        .adapter(Arc::new(adapter(&server)))
        .router(Arc::new(router))
        .retry_policy(policy)
        .build()
        .expect("build");
    let resp = gw
        .call(request(), CancellationToken::new())
        .await
        .expect("served after retry");
    assert_eq!(resp.attempts, 2);
    assert_eq!(server.received_requests().await.expect("recorded").len(), 2);
}

#[tokio::test]
async fn anthropic_529_alone_is_transient_overloaded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529).set_body_json(json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})))
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request()).await.expect_err("529");
    assert!(matches!(
        err,
        GatewayError::Transient {
            kind: TransientKind::Overloaded,
            ..
        }
    ));
}

#[tokio::test]
async fn anthropic_401_permanent_auth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}})))
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request()).await.expect_err("401");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::Auth,
            ..
        }
    ));
}

#[tokio::test]
async fn anthropic_invalid_json_200_is_truncated_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"content\": ["))
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request())
        .await
        .expect_err("bad json");
    assert!(matches!(
        err,
        GatewayError::Transient {
            kind: TransientKind::TruncatedBody,
            ..
        }
    ));
}

#[tokio::test]
async fn anthropic_unexpected_shape_is_permanent_unknown() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content": "not an array"})))
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request())
        .await
        .expect_err("shape");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::Unknown,
            ..
        }
    ));
}

#[test]
fn anthropic_not_registered_without_key() {
    let none = AnthropicAdapter::from_lookup(|_| None).expect("ok");
    assert!(none.is_none());
    let blank =
        AnthropicAdapter::from_lookup(|k| (k == "ANTHROPIC_API_KEY").then(|| "  ".to_owned()))
            .expect("ok");
    assert!(blank.is_none());
    let some = AnthropicAdapter::from_lookup(|k| {
        (k == "ANTHROPIC_API_KEY").then(|| "sk-ant-x".to_owned())
    })
    .expect("ok");
    assert!(some.is_some());
}

#[test]
fn anthropic_base_url_override_must_be_https_in_production() {
    let lookup = |k: &str| match k {
        "ANTHROPIC_API_KEY" => Some("sk-ant-x".to_owned()),
        "ANTHROPIC_BASE_URL" => Some("http://evil.example".to_owned()),
        "RG_ENV" => Some("production".to_owned()),
        _ => None,
    };
    assert!(AnthropicAdapter::from_lookup(lookup).is_err());
}

#[test]
fn anthropic_debug_never_prints_api_key() {
    let a = AnthropicAdapter::new(Secret::new("sk-ant-supersecret"), None).expect("adapter");
    assert!(!format!("{a:?}").contains("supersecret"));
}
