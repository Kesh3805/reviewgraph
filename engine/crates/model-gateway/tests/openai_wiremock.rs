#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod common;

use std::time::Duration;

use model_gateway::adapters::openai::OpenAiAdapter;
use model_gateway::adapters::openai_wire::build_request;
use model_gateway::{
    check_strict_compatible, FinishReason, GatewayError, ModelOutput, OutputSchema, PermanentKind,
    ProviderAdapter, ProviderId, ProviderRequest, ReasoningLevel, RouteCandidate,
};
use serde_json::{json, Value};
use telemetry::Secret;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::request;

fn candidate(reasoning: bool) -> RouteCandidate {
    RouteCandidate {
        provider: ProviderId::new("openai"),
        model: "test-model".into(),
        max_context: 128_000,
        supports_reasoning: reasoning,
    }
}

fn adapter(server: &MockServer) -> OpenAiAdapter {
    OpenAiAdapter::new(Secret::new("sk-test-key-123456"), Some(server.uri())).expect("adapter")
}

async fn send(
    a: &OpenAiAdapter,
    req: &model_gateway::ModelRequest,
) -> Result<model_gateway::ProviderResponse, GatewayError> {
    let c = candidate(false);
    let h = model_gateway::request_hash(req);
    a.send(&ProviderRequest {
        request_hash: &h,
        request: req,
        candidate: &c,
        timeout: Duration::from_secs(5),
    })
    .await
}

fn ok_body() -> Value {
    json!({
        "id": "resp_1", "status": "completed", "model": "test-model",
        "output": [
            {"type": "reasoning", "summary": []},
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "{\"ok\":true}"}]}
        ],
        "usage": {
            "input_tokens": 5000, "input_tokens_details": {"cached_tokens": 4096},
            "output_tokens": 70, "output_tokens_details": {"reasoning_tokens": 50}
        }
    })
}

#[test]
fn openai_builds_strict_json_schema_request() {
    let body = build_request(&request(), &candidate(false));
    insta::assert_json_snapshot!(body);
}

#[test]
fn openai_always_sets_store_false() {
    for reasoning in [true, false] {
        let body = build_request(
            &request().with_reasoning(ReasoningLevel::High),
            &candidate(reasoning),
        );
        assert_eq!(body["store"], json!(false));
    }
}

#[test]
fn openai_reasoning_only_when_supported() {
    let req = request().with_reasoning(ReasoningLevel::Medium);
    assert_eq!(
        build_request(&req, &candidate(true))["reasoning"]["effort"],
        "medium"
    );
    assert!(build_request(&req, &candidate(false))
        .get("reasoning")
        .is_none());
}

#[tokio::test]
async fn openai_parses_output_text_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer sk-test-key-123456"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.output, ModelOutput::Json(json!({"ok": true})));
    assert_eq!(r.finish_reason, FinishReason::Complete);
    let sent: Value =
        serde_json::from_slice(&server.received_requests().await.expect("rec")[0].body)
            .expect("json");
    assert_eq!(sent["store"], json!(false));
    assert_eq!(sent["text"]["format"]["strict"], json!(true));
}

#[tokio::test]
async fn openai_usage_subtracts_cached_from_input() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.usage.input_uncached, 904);
    assert_eq!(r.usage.cache_read, 4096);
    assert_eq!(r.usage.cache_write, 0);
    assert_eq!(r.usage.output, 70);
    assert_eq!(r.usage.reasoning, 50);
}

#[tokio::test]
async fn openai_incomplete_max_output_tokens() {
    let server = MockServer::start().await;
    let body = json!({
        "status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"}, "model": "m",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "{\"ok\": tr"}]}],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    });
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.finish_reason, FinishReason::MaxTokens);
}

#[tokio::test]
async fn openai_refusal_part_maps_to_refusal() {
    let server = MockServer::start().await;
    let body = json!({
        "status": "completed", "model": "m",
        "output": [{"type": "message", "content": [{"type": "refusal", "refusal": "no"}]}],
        "usage": {"input_tokens": 1, "output_tokens": 1}
    });
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let r = send(&adapter(&server), &request()).await.expect("ok");
    assert_eq!(r.finish_reason, FinishReason::Refusal);
}

#[tokio::test]
async fn openai_unparseable_output_is_schema_violation() {
    let server = MockServer::start().await;
    let body = json!({
        "status": "completed", "model": "m",
        "output": [{"type": "message", "content": [{"type": "output_text", "text": "not json"}]}],
        "usage": {"input_tokens": 1, "output_tokens": 1}
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
async fn openai_no_message_item_is_permanent_unknown() {
    let server = MockServer::start().await;
    let body = json!({"status": "completed", "model": "m", "output": [{"type": "reasoning"}], "usage": {}});
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request())
        .await
        .expect_err("no message");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::Unknown,
            ..
        }
    ));
}

#[tokio::test]
async fn openai_insufficient_quota_is_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({"error": {"type": "insufficient_quota", "code": "insufficient_quota", "message": "You exceeded your current quota"}})))
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request())
        .await
        .expect_err("quota");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::QuotaExhausted,
            ..
        }
    ));
}

#[tokio::test]
async fn openai_429_rate_limited() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "3")
                .set_body_json(json!({"error": {"type": "rate_limit_exceeded", "code": "rate_limit_exceeded", "message": "Rate limit reached for requests"}})),
        )
        .mount(&server)
        .await;
    let err = send(&adapter(&server), &request()).await.expect_err("429");
    match err {
        GatewayError::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(Duration::from_secs(3)))
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn openai_refuses_non_strict_schema_without_io() {
    let server = MockServer::start().await;
    let mut req = request();
    req.output_schema = Some(OutputSchema::new(
        "loose",
        "1",
        json!({"type": "object", "properties": {"a": {"type": "string"}}}),
    ));
    let err = send(&adapter(&server), &req).await.expect_err("non strict");
    assert!(matches!(
        err,
        GatewayError::Permanent {
            kind: PermanentKind::UnsupportedParameter,
            ..
        }
    ));
    assert!(err.fallback_eligible());
    assert!(server.received_requests().await.expect("rec").is_empty());
}

#[test]
fn strict_checker_rejects_missing_required() {
    let schema = json!({"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "string"}}, "required": ["a"], "additionalProperties": false});
    let errs = check_strict_compatible(&schema).expect_err("missing b");
    assert!(errs.iter().any(|e| e.contains("properties/b")), "{errs:?}");
}

#[test]
fn strict_checker_rejects_open_additional_properties() {
    let schema = json!({"type": "object", "properties": {"a": {"type": "object", "properties": {}, "required": []}}, "required": ["a"], "additionalProperties": false});
    let errs = check_strict_compatible(&schema).expect_err("nested open");
    assert!(
        errs.iter()
            .any(|e| e.contains("properties/a") && e.contains("additionalProperties")),
        "{errs:?}"
    );
}

#[test]
fn strict_checker_accepts_nullable_optional_pattern() {
    let schema = json!({
        "type": "object",
        "properties": {"a": {"type": ["string", "null"]}, "list": {"type": "array", "items": {"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"], "additionalProperties": false}}},
        "required": ["a", "list"], "additionalProperties": false
    });
    assert!(check_strict_compatible(&schema).is_ok());
}

#[test]
fn openai_not_registered_without_key() {
    assert!(OpenAiAdapter::from_lookup(|_| None).expect("ok").is_none());
    let some = OpenAiAdapter::from_lookup(|k| (k == "OPENAI_API_KEY").then(|| "sk-x".to_owned()))
        .expect("ok");
    assert!(some.is_some());
}

#[test]
fn openai_debug_never_prints_api_key() {
    let a = OpenAiAdapter::new(Secret::new("sk-supersecret"), None).expect("adapter");
    assert!(!format!("{a:?}").contains("supersecret"));
}
