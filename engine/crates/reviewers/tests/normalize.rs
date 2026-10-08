#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! REV-C-003: candidate normalisation.

mod common;

use std::sync::Arc;
use std::time::Duration;

use model_gateway::adapters::replay::{ReplayAdapter, ReplayConfig};
use model_gateway::fixture::{Fixture, FixtureResponse, FixtureStore, FIXTURE_VERSION};
use model_gateway::{
    request_hash, DefaultRedactor, FinishReason, GatewayBuilder, ModelOutput, ModelTier,
    OutputValidator, ProviderId, RouteCandidate, StaticRouter, TraceContext, Usage,
};
use review_core::finding::{FindingCategory, Severity};
use review_core::ids::SymbolKey;
use review_core::location::DiffSide;
use reviewers::claim_text;
use reviewers::normalize::RejectionCode;
use reviewers::output::{parse_output, RawItem};
use reviewers::{
    normalize, normalize_all, CorrectnessReviewer, RefTable, RefValidator, Reviewer, ReviewerKind,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use common::{auth_bypass_context, golden_output, request, risk, AUTHORIZE, UPDATE_USER};

fn refs() -> RefTable {
    RefTable::build(&auth_bypass_context())
}

fn item(mutate: impl FnOnce(&mut Value)) -> RawItem {
    let mut out = golden_output();
    mutate(&mut out["findings"][0]);
    let (mut items, _, dropped) = parse_output(&out);
    assert_eq!(dropped, 0);
    items.remove(0)
}

#[test]
fn resolves_anchor_ref_to_symbol_key() {
    let c = normalize(&item(|_| {}), &refs(), ReviewerKind::Correctness).unwrap();
    assert_eq!(c.anchor.symbol_id.as_str(), AUTHORIZE);
    assert_eq!(c.anchor.symbol_key, SymbolKey::of(&c.anchor.symbol_id));
    assert_eq!(c.anchor.side, DiffSide::Head);
    assert_eq!((c.anchor.start_line, c.anchor.end_line), (12, 15));
    assert_eq!(c.category, FindingCategory::Security);
    assert_eq!(c.severity, Severity::High);
    assert_eq!(c.affected_symbols[0].as_str(), AUTHORIZE);
    assert_eq!(c.affected_symbols[1].as_str(), UPDATE_USER);
    assert_eq!(c.linked_items, ["T1"]);
    assert!(c.notes.is_empty(), "{:?}", c.notes);
}

#[test]
fn clamps_anchor_to_symbol_range() {
    let raw = item(|f| {
        f["anchor"]["start_line"] = 30.into();
        f["anchor"]["end_line"] = 32.into();
    });
    let c = normalize(&raw, &refs(), ReviewerKind::Correctness).unwrap();
    assert_eq!((c.anchor.start_line, c.anchor.end_line), (17, 17));
    assert_eq!(c.notes, ["anchor_clamped"]);
}

#[test]
fn rejects_line_outside_file() {
    let raw = item(|f| {
        f["anchor"]["start_line"] = 120.into();
        f["anchor"]["end_line"] = 121.into();
    });
    let r = normalize(&raw, &refs(), ReviewerKind::Correctness).unwrap_err();
    assert_eq!(r.code, RejectionCode::LineOutOfRange);
}

#[test]
fn rejects_when_all_evidence_unresolvable() {
    let raw = item(|f| {
        f["evidence"][0]["ref"] = "S9".into();
        f["evidence"][1]["ref"] = "N7".into();
    });
    let r = normalize(&raw, &refs(), ReviewerKind::Correctness).unwrap_err();
    assert_eq!(r.code, RejectionCode::EvidenceMissing);
    assert_eq!(r.ordinal, 0);
    assert!(!r.raw_output_hash.is_empty());
}

#[test]
fn drops_unresolvable_relation_with_note() {
    let raw = item(|f| {
        f["claimed_relations"][1]["from"] = "N9".into();
    });
    let c = normalize(&raw, &refs(), ReviewerKind::Correctness).unwrap();
    assert_eq!(c.claimed_relations.len(), 1);
    assert_eq!(c.claimed_relations[0].from, UPDATE_USER);
    assert_eq!(c.claimed_relations[0].to, AUTHORIZE);
    assert!(c.notes.contains(&"relation_dropped".to_owned()));
}

#[test]
fn predicate_params_resolved_to_symbol_ids() {
    let raw = item(|f| {
        f["predicate"]["params"] = json!([
            {"name": "caller", "value": "N1"},
            {"name": "callee", "value": "PermissionService.check"}
        ]);
    });
    let c = normalize(&raw, &refs(), ReviewerKind::Correctness).unwrap();
    assert_eq!(c.predicate.subject, AUTHORIZE);
    assert_eq!(c.predicate.params["caller"], UPDATE_USER);
    assert_eq!(c.predicate.params["callee"], "PermissionService.check");
}

#[test]
fn path_comes_from_ref_table_not_model() {
    let raw = item(|f| {
        f["description"] = "See ../../etc/passwd and src/other.ts".into();
    });
    let c = normalize(&raw, &refs(), ReviewerKind::Correctness).unwrap();
    assert_eq!(c.anchor.path, "src/auth/auth.service.ts");
    assert_eq!(c.evidence[1].path, "src/admin/admin.service.ts");
}

#[test]
fn claim_normalization_strips_hedges_and_refs() {
    let t = claim_text::normalize(
        "S1 **might** possibly   skip the `check`, and N1 could write!",
        &refs(),
    );
    assert_eq!(
        t,
        format!("{AUTHORIZE} skip the check and {UPDATE_USER} write")
    );
}

#[test]
fn rejected_candidates_are_persisted_with_reason() {
    let mut out = golden_output();
    let mut bad = out["findings"][0].clone();
    bad["anchor"]["ref"] = "N9".into();
    out["findings"].as_array_mut().unwrap().push(bad);
    let (items, _, _) = parse_output(&out);
    let (ok, rejected) = normalize_all(&items, &refs(), ReviewerKind::Correctness);
    assert_eq!(ok.len(), 1);
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].code, RejectionCode::UnresolvableReference);
    assert_eq!(rejected[0].ordinal, 1);
    assert_eq!(rejected[0].raw_json["anchor"]["ref"], "N9");
    // Deterministic: the same raw item hashes the same.
    let (_, again) = normalize_all(&items, &refs(), ReviewerKind::Correctness);
    assert_eq!(again[0].raw_output_hash, rejected[0].raw_output_hash);
}

#[test]
fn ref_validator_reports_unknown_refs() {
    let v = RefValidator::new(Arc::new(refs()));
    assert!(v.validate(&golden_output()).is_empty());
    let mut out = golden_output();
    out["findings"][0]["anchor"]["ref"] = "S7".into();
    out["findings"][0]["affected_refs"] = json!(["T4"]);
    let errors = v.validate(&out);
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].instance_path, "/findings/0/anchor/ref");
    for e in &errors {
        assert!(!e.message.contains("S7") && !e.message.contains("T4"));
    }
}

fn fixture(req: &model_gateway::ModelRequest, output: Value) -> Fixture {
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
            usage: Usage::default(),
            finish_reason: FinishReason::Complete,
        },
        latency_ms: 0,
    }
}

/// The ref validator runs inside the gateway: a hallucinated ref gets one repair turn.
#[tokio::test]
async fn ref_validator_triggers_repair_for_unknown_ref() {
    let cx = auth_bypass_context();
    let r = risk();
    let trace = TraceContext::default();
    let mut req = request(&cx, &r, &[], &trace);
    req.budget = model_gateway::CallBudget::within(Duration::from_secs(60));
    let reviewer = CorrectnessReviewer::new().unwrap();
    let (model_req, refs) = reviewer
        .build_request(&req, ModelTier::ReviewReasoner)
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FixtureStore::new(dir.path()));
    let mut hallucinated = golden_output();
    hallucinated["findings"][0]["evidence"][1]["ref"] = "N8".into();
    store
        .write(
            model_req.task,
            &fixture(&model_req, hallucinated.clone()),
            false,
        )
        .await
        .unwrap();
    let errors = RefValidator::new(refs).validate(&hallucinated);
    let mut repaired = model_req.clone();
    repaired.input.repair = Some(model_gateway::RepairTurn {
        previous_output: hallucinated,
        errors: model_gateway::validate::cap_errors(errors),
        tool_use_id: None,
    });
    store
        .write(model_req.task, &fixture(&repaired, golden_output()), false)
        .await
        .unwrap();

    let mut cfg = ReplayConfig::new(dir.path());
    cfg.miss_log = None;
    let gw = GatewayBuilder::new()
        .redactor(Arc::new(DefaultRedactor::new()))
        .adapter(Arc::new(ReplayAdapter::new(
            ProviderId::new("anthropic"),
            store,
            cfg,
        )))
        .router(Arc::new(StaticRouter::new().with_route(
            ModelTier::ReviewReasoner,
            vec![RouteCandidate {
                provider: ProviderId::new("anthropic"),
                model: "m".into(),
                max_context: 200_000,
                supports_reasoning: false,
            }],
        )))
        .build()
        .unwrap();
    let out = reviewer
        .review(req, &gw, CancellationToken::new())
        .await
        .expect("served after one repair");
    assert_eq!(out.model_response_meta.attempts, 2);
    let (ok, rejected) = normalize_all(&out.raw, &out.ref_table, ReviewerKind::Correctness);
    assert_eq!((ok.len(), rejected.len()), (1, 0));
}
