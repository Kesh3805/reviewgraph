//! REV-T-002: a behaviourally changed public symbol with no `TESTS` coverage.

use super::{
    behaviourally_changed, inbound, is_test_path, DeterministicFinding, GraphAbsence,
    PrecheckOutcome, SeverityHint,
};
use crate::context::{ChangedSymbol, ReviewContext};

pub const RULE_ID: &str = "tests.public_symbol_untested";
/// Minimum `TESTS` edge confidence that counts as coverage.
pub const MIN_TESTS_CONFIDENCE: f64 = 0.6;
/// Callers above which an untested change is medium severity.
const MANY_CALLERS: usize = 5;

/// Runs the pre-check. `test_framework_detected` comes from INIT; without a framework the check
/// reports `NotExecuted`.
pub fn tests_precheck(cx: &dyn ReviewContext, test_framework_detected: bool) -> PrecheckOutcome {
    if !test_framework_detected {
        return PrecheckOutcome::NotExecuted {
            reason: "no test framework detected; test mapping is unavailable".into(),
        };
    }
    let findings = cx
        .changed_symbols()
        .iter()
        .filter(|s| behaviourally_changed(s) && !is_test_path(&s.path) && is_public(cx, s))
        .filter(|s| !covered(cx, s))
        .map(|s| finding(cx, s))
        .collect();
    PrecheckOutcome::Executed {
        findings,
        truncated: false,
    }
}

fn is_endpoint_handler(cx: &dyn ReviewContext, s: &ChangedSymbol) -> bool {
    ["ROUTES_TO", "HANDLED_BY"]
        .iter()
        .any(|k| inbound(cx.edges(), &s.symbol_id, k).next().is_some())
        || s.body_head.as_deref().is_some_and(|b| {
            ["@Get(", "@Post(", "@Put(", "@Patch(", "@Delete("]
                .iter()
                .any(|d| b.contains(d))
        })
}

fn is_public(cx: &dyn ReviewContext, s: &ChangedSymbol) -> bool {
    if is_endpoint_handler(cx, s) {
        return true;
    }
    let head = s
        .signature_head
        .as_deref()
        .or(s.body_head.as_deref())
        .unwrap_or("")
        .trim_start();
    if head.starts_with("private ") || head.starts_with("protected ") || head.starts_with('#') {
        return false;
    }
    let name = s.symbol_id.rsplit(['.', '#']).next().unwrap_or("");
    let name = name.split('/').next().unwrap_or(name);
    !name.starts_with('_')
}

fn name_of(s: &ChangedSymbol) -> &str {
    let tail = s.symbol_id.rsplit(['.', '#']).next().unwrap_or("");
    tail.split('/').next().unwrap_or(tail)
}

/// Coverage: a mapped test (direct or one hop through a wrapper), or a test file touched by the
/// PR that mentions the symbol.
fn covered(cx: &dyn ReviewContext, s: &ChangedSymbol) -> bool {
    let id = s.symbol_id.as_str();
    let direct = |sym: &str| {
        cx.tests().iter().any(|t| t.covers.iter().any(|c| c == sym))
            || inbound(cx.edges(), sym, "TESTS").any(|e| e.confidence >= MIN_TESTS_CONFIDENCE)
    };
    if direct(id) {
        return true;
    }
    // One hop: a wrapper that calls the symbol is itself covered.
    if inbound(cx.edges(), id, "CALLS").any(|e| direct(&e.from)) {
        return true;
    }
    let name = name_of(s);
    !name.is_empty()
        && cx.changed_symbols().iter().any(|t| {
            is_test_path(&t.path) && t.body_head.as_deref().is_some_and(|b| b.contains(name))
        })
}

fn finding(cx: &dyn ReviewContext, s: &ChangedSymbol) -> DeterministicFinding {
    let callers = inbound(cx.edges(), &s.symbol_id, "CALLS").count();
    let severity_hint = if is_endpoint_handler(cx, s) || callers > MANY_CALLERS {
        SeverityHint::Medium
    } else {
        SeverityHint::Low
    };
    DeterministicFinding {
        rule_id: RULE_ID.into(),
        reviewer: "deterministic:tests".into(),
        symbol_key: s.symbol_id.clone(),
        path: s.path.clone(),
        anchor_range: s.range,
        severity_hint,
        message: format!("`{}` changed behaviour and no test maps to it", name_of(s)),
        absence: Some(GraphAbsence {
            edge_kind: "TESTS".into(),
            min_confidence: MIN_TESTS_CONFIDENCE,
            searched_depth: 1,
        }),
        members: Vec::new(),
    }
}
