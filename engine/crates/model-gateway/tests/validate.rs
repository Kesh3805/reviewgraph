#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! GW-009: output JSON-schema validation and one repair retry.

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use model_gateway::adapters::replay::{ReplayAdapter, ReplayConfig};
use model_gateway::fixture::{Fixture, FixtureResponse, FixtureStore, FIXTURE_VERSION};
use model_gateway::validate::cap_errors;
use model_gateway::{
    request_hash, Error, FinishReason, GatewayBuilder, GatewayError, MemoryLedger, ModelGateway,
    ModelOutput, ModelRequest, ModelTier, OutputSchema, OutputValidator, ProviderAdapter,
    ProviderId, ProviderRequest, ProviderResponse, RepairTurn, RouteCandidate, SchemaErrorSummary,
    SchemaValidators, ServedFrom, StaticRouter, Usage,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use common::{request, schema, TestMetrics};

fn candidate() -> RouteCandidate {
    RouteCandidate {
        provider: ProviderId::new("anthropic"),
        model: "claude-sonnet-5-5".into(),
        max_context: 200_000,
        supports_reasoning: false,
    }
}

fn reply(output: Value, finish: FinishReason) -> ProviderResponse {
    ProviderResponse {
        output: ModelOutput::Json(output),
        usage: Usage {
            input_uncached: 10,
            output: 5,
            ..Usage::default()
        },
        finish_reason: finish,
        model: "claude-sonnet-5-5".into(),
        provider_request_id: None,
        tool_use_id: Some("toolu_1".into()),
        served_from: ServedFrom::Live,
    }
}

/// Answers from a queue and remembers whether each request carried a repair turn.
struct Script {
    replies: Mutex<VecDeque<ProviderResponse>>,
    seen: Arc<Mutex<Vec<Option<RepairTurn>>>>,
}

#[async_trait]
impl ProviderAdapter for Script {
    fn provider(&self) -> ProviderId {
        ProviderId::new("anthropic")
    }

    async fn send(&self, req: &ProviderRequest<'_>) -> Result<ProviderResponse, GatewayError> {
        self.seen
            .lock()
            .unwrap()
            .push(req.request.input.repair.clone());
        Ok(self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted reply"))
    }
}

struct Rig {
    gw: model_gateway::Gateway,
    seen: Arc<Mutex<Vec<Option<RepairTurn>>>>,
    metrics: TestMetrics,
}

fn rig(replies: Vec<ProviderResponse>) -> Rig {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let metrics = TestMetrics::new();
    let gw = GatewayBuilder::new()
        .redactor(Arc::new(model_gateway::DefaultRedactor::new()))
        .adapter(Arc::new(Script {
            replies: Mutex::new(replies.into()),
            seen: seen.clone(),
        }))
        .router(Arc::new(
            StaticRouter::new().with_route(ModelTier::ReviewReasoner, vec![candidate()]),
        ))
        .metrics(metrics.metrics.clone())
        .build()
        .expect("build");
    Rig { gw, seen, metrics }
}

async fn call(r: &Rig, req: ModelRequest) -> Result<model_gateway::ModelResponse, GatewayError> {
    r.gw.call(req, CancellationToken::new()).await
}

#[tokio::test]
async fn valid_output_passes_without_repair() {
    let r = rig(vec![reply(json!({"ok": true}), FinishReason::Complete)]);
    let resp = call(&r, request()).await.expect("valid");
    assert_eq!(resp.attempts, 1);
    assert_eq!(r.seen.lock().unwrap().len(), 1);
    assert_eq!(r.metrics.sum("structured_output_success_total"), 1);
    assert_eq!(r.metrics.sum("llm_schema_repairs_total"), 0);
}

#[tokio::test]
async fn invalid_output_triggers_exactly_one_repair() {
    let r = rig(vec![
        reply(json!({"ok": "yes"}), FinishReason::Complete),
        reply(json!({"ok": true}), FinishReason::Complete),
    ]);
    let resp = call(&r, request()).await.expect("repaired");
    assert_eq!(resp.output, ModelOutput::Json(json!({"ok": true})));
    assert_eq!(resp.attempts, 2);
    // Usage of both calls is billed to the logical call.
    assert_eq!(resp.usage.output, 10);
    let seen = r.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].is_none());
    let repair = seen[1].as_ref().expect("repair turn");
    assert_eq!(repair.previous_output, json!({"ok": "yes"}));
    assert_eq!(repair.errors[0].instance_path, "/ok");
    assert_eq!(repair.errors[0].keyword, "type");
    assert_eq!(repair.tool_use_id.as_deref(), Some("toolu_1"));
    assert_eq!(r.metrics.sum("llm_schema_repairs_total"), 1);
    assert_eq!(r.metrics.sum("structured_output_success_total"), 1);
    assert_eq!(r.metrics.sum("structured_output_failures_total"), 1);
}

#[tokio::test]
async fn second_failure_returns_schema_violation() {
    let r = rig(vec![
        reply(json!({"ok": "yes"}), FinishReason::Complete),
        reply(json!({"ok": "still no"}), FinishReason::Complete),
    ]);
    let err = call(&r, request()).await.expect_err("invalid twice");
    match err {
        GatewayError::SchemaViolation { errors, repaired } => {
            assert!(repaired);
            assert_eq!(errors[0].instance_path, "/ok");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(r.seen.lock().unwrap().len(), 2);
    assert_eq!(r.metrics.sum("structured_output_failures_total"), 2);
    assert_eq!(r.metrics.sum("structured_output_success_total"), 0);
}

#[tokio::test]
async fn max_tokens_skips_repair() {
    let r = rig(vec![reply(json!({"ok": "trunc"}), FinishReason::MaxTokens)]);
    let err = call(&r, request()).await.expect_err("truncated");
    assert!(matches!(
        err,
        GatewayError::SchemaViolation {
            repaired: false,
            ..
        }
    ));
    assert_eq!(r.seen.lock().unwrap().len(), 1);
    assert_eq!(r.metrics.sum("llm_schema_repairs_total"), 0);
}

/// Rejects `ok == false` (a stand-in for a semantic check such as ref existence).
#[derive(Debug)]
struct MustBeTrue;

impl OutputValidator for MustBeTrue {
    fn validate(&self, output: &Value) -> Vec<SchemaErrorSummary> {
        if output.get("ok") == Some(&Value::Bool(true)) {
            Vec::new()
        } else {
            vec![SchemaErrorSummary {
                instance_path: "/ok".into(),
                keyword: "semantic".into(),
                message: "ok must be true".into(),
            }]
        }
    }
}

#[tokio::test]
async fn semantic_validator_errors_trigger_repair() {
    let r = rig(vec![
        reply(json!({"ok": false}), FinishReason::Complete),
        reply(json!({"ok": true}), FinishReason::Complete),
    ]);
    let req = request().with_validator(Arc::new(MustBeTrue));
    let resp = call(&r, req).await.expect("repaired");
    assert_eq!(resp.attempts, 2);
    let seen = r.seen.lock().unwrap();
    let repair = seen[1].as_ref().expect("repair");
    assert_eq!(repair.errors[0].keyword, "semantic");
}

fn repaired_request() -> ModelRequest {
    let mut req = request();
    req.input.repair = Some(RepairTurn {
        previous_output: json!({"ok": "yes"}),
        errors: vec![SchemaErrorSummary {
            instance_path: "/ok".into(),
            keyword: "type".into(),
            message: "value violates keyword `type`".into(),
        }],
        tool_use_id: Some("toolu_1".into()),
    });
    req
}

#[test]
fn repair_turn_rendered_for_anthropic() {
    let body = model_gateway::adapters::anthropic_wire::build_request(
        &repaired_request(),
        "claude-sonnet-5-5",
    );
    insta::assert_json_snapshot!(body["messages"]);
}

#[test]
fn repair_turn_rendered_for_openai() {
    let c = RouteCandidate {
        provider: ProviderId::new("openai"),
        model: "test-model".into(),
        max_context: 128_000,
        supports_reasoning: false,
    };
    let body = model_gateway::adapters::openai_wire::build_request(&repaired_request(), &c);
    insta::assert_json_snapshot!(body["input"]);
}

#[test]
fn repair_turn_is_part_of_request_hash() {
    assert_ne!(request_hash(&request()), request_hash(&repaired_request()));
}

#[test]
fn error_summary_has_no_instance_values() {
    let s = OutputSchema::new(
        "t",
        "1",
        json!({
            "type": "object", "additionalProperties": false, "required": ["a", "b", "c"],
            "properties": {
                "a": {"type": "integer"},
                "b": {"enum": ["x", "y"]},
                "c": {"type": "string", "maxLength": 3}
            }
        }),
    );
    let out = json!({"a": "CANARY-ONE", "b": "CANARY-TWO", "c": "CANARY-THREE"});
    let errors = SchemaValidators::new()
        .validate(&s, &out)
        .expect("compiles");
    assert_eq!(errors.len(), 3);
    for e in &errors {
        let text = format!("{} {} {}", e.instance_path, e.keyword, e.message);
        assert!(!text.contains("CANARY"), "{text}");
    }
    let rendered = model_gateway::validate::render_repair_errors(&errors);
    assert!(!rendered.contains("CANARY"), "{rendered}");
}

#[tokio::test]
async fn repair_counts_against_budget() {
    // With a single attempt allowed, there is no budget left for a repair.
    let r = rig(vec![reply(json!({"ok": "yes"}), FinishReason::Complete)]);
    let mut req = request();
    req.budget.max_attempts = 1;
    let err = call(&r, req).await.expect_err("no budget for repair");
    assert!(matches!(
        err,
        GatewayError::SchemaViolation {
            repaired: false,
            ..
        }
    ));
    assert_eq!(r.seen.lock().unwrap().len(), 1);

    // With the default budget the repair is the second attempt.
    let r = rig(vec![
        reply(json!({"ok": "yes"}), FinishReason::Complete),
        reply(json!({"ok": true}), FinishReason::Complete),
    ]);
    assert_eq!(call(&r, request()).await.expect("ok").attempts, 2);
}

#[test]
fn invalid_schema_fails_at_startup_selftest() {
    let res = GatewayBuilder::new()
        .redactor(Arc::new(model_gateway::DefaultRedactor::new()))
        .adapter(Arc::new(Script {
            replies: Mutex::new(VecDeque::new()),
            seen: Arc::new(Mutex::new(Vec::new())),
        }))
        .router(Arc::new(StaticRouter::new()))
        .register_schema(schema())
        .register_schema(OutputSchema::new("broken", "1", json!({"type": 5})))
        .build();
    match res {
        Err(Error::Config(msg)) => assert!(msg.contains("broken"), "{msg}"),
        other => panic!("expected a config error, got {other:?}"),
    }
}

fn fixture(req: &ModelRequest, output: Value) -> Fixture {
    Fixture {
        fixture_version: FIXTURE_VERSION,
        request_hash: request_hash(req).0,
        provider: "any".into(),
        model: "any".into(),
        task: req.task.as_str().to_owned(),
        prompt_id: req.input.system.prompt_id.clone(),
        prompt_version: req.input.system.prompt_version.clone(),
        schema_hash: req.output_schema.as_ref().map(|s| s.hash.clone()),
        synthetic: true,
        recorded_at: "2026-10-08T00:00:00Z".into(),
        engine_git_sha: None,
        response: FixtureResponse {
            output: ModelOutput::Json(output),
            usage: Usage {
                input_uncached: 100,
                output: 20,
                ..Usage::default()
            },
            finish_reason: FinishReason::Complete,
        },
        latency_ms: 0,
    }
}

/// Acceptance: under replay fixtures with one malformed and one repaired output, the gateway
/// writes exactly two ledger rows and counts one structured-output success.
#[tokio::test]
async fn replay_malformed_then_repaired_writes_two_ledger_rows() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = Arc::new(FixtureStore::new(dir.path()));
    let req = request();
    let bad = json!({"ok": "yes"});
    store
        .write(req.task, &fixture(&req, bad.clone()), false)
        .await
        .expect("write");
    // The repair request exactly as the gateway builds it (replay responses carry no tool id).
    let errors = cap_errors(
        SchemaValidators::new()
            .validate(&schema(), &bad)
            .expect("compiles"),
    );
    let mut repaired = req.clone();
    repaired.input.repair = Some(RepairTurn {
        previous_output: bad,
        errors,
        tool_use_id: None,
    });
    store
        .write(req.task, &fixture(&repaired, json!({"ok": true})), false)
        .await
        .expect("write");

    let mut cfg = ReplayConfig::new(dir.path());
    cfg.miss_log = None;
    let ledger = Arc::new(MemoryLedger::new());
    let metrics = TestMetrics::new();
    let gw = GatewayBuilder::new()
        .redactor(Arc::new(model_gateway::DefaultRedactor::new()))
        .adapter(Arc::new(ReplayAdapter::new(
            ProviderId::new("anthropic"),
            store,
            cfg,
        )))
        .router(Arc::new(
            StaticRouter::new().with_route(ModelTier::ReviewReasoner, vec![candidate()]),
        ))
        .ledger(ledger.clone())
        .metrics(metrics.metrics.clone())
        .build()
        .expect("build");
    let resp = gw
        .call(req, CancellationToken::new())
        .await
        .expect("repaired under replay");
    assert_eq!(resp.output, ModelOutput::Json(json!({"ok": true})));
    let rows = ledger.records();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].request_hash, rows[1].request_hash);
    assert_eq!(rows[1].attempt, 2);
    assert_eq!(metrics.sum("structured_output_success_total"), 1);
}
