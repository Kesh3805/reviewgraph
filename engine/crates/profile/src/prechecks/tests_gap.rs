//! Changed public symbol with zero `TESTS` edges (REV-T-002).
//!
//! Flags each behaviorally changed symbol that is exported/public or an endpoint handler, has no
//! inbound `TESTS` edge with confidence ≥ 0.6 (directly, or through a caller one hop away that is
//! tested), and is not covered by a test file the pull request adds or changes. A touched test
//! file covers a symbol when it imports the symbol's file or names the symbol in a suite or case
//! title. Severity is medium for endpoint handlers and symbols with more than five callers, low
//! otherwise.

use std::collections::BTreeSet;

use analysis_ir::symbol::Visibility;
use codegraph::{Direction, EdgeKind, GraphQuery, NodeFlags, NodeKind, NodeRef};
use review_core::evidence::EvidenceKind;

use super::{DeterministicFinding, PrecheckEvidence, PrecheckOutcome, SeverityHint};
use crate::conventions::builtin::edges;
use crate::conventions::builtin::testing::MIN_TESTS_CONFIDENCE;
use crate::infer::roles::is_test_path;
use crate::policy::change::{ChangeSet, ChangedSymbol, FileStatus, SymbolChange};
use crate::policy::evidence::{deterministic, location, symbol_ref};

pub const RULE_ID: &str = "tests.public_symbol_untested";
/// More callers than this make an untested change medium severity.
const MANY_CALLERS: usize = 5;

/// Whether a symbol is part of the public surface: exported itself, or a public member of an
/// exported class.
pub fn is_public(graph: &dyn GraphQuery, node: &NodeRef<'_>) -> bool {
    if node.attrs.flags.contains(NodeFlags::EXPORTED) {
        return true;
    }
    if node.attrs.visibility != Visibility::Public {
        return false;
    }
    node.attrs
        .parent
        .and_then(|p| graph.node(p))
        .is_some_and(|parent| parent.attrs.flags.contains(NodeFlags::EXPORTED))
}

/// Whether the symbol handles an API endpoint.
pub fn is_endpoint_handler(graph: &dyn GraphQuery, node: &NodeRef<'_>) -> bool {
    edges(graph, node.key, Direction::In, &[EdgeKind::HandledBy])
        .iter()
        .filter_map(|e| graph.node(e.source))
        .any(|source| source.kind == NodeKind::ApiEndpoint)
}

fn is_tested(graph: &dyn GraphQuery, key: codegraph::NodeKey) -> bool {
    edges(graph, key, Direction::In, &[EdgeKind::Tests])
        .iter()
        .any(|e| e.confidence.as_f32() >= MIN_TESTS_CONFIDENCE)
}

/// Tested directly, or through a tested caller one hop away (a thin wrapper).
pub fn has_test_coverage(graph: &dyn GraphQuery, node: &NodeRef<'_>) -> bool {
    if is_tested(graph, node.key) {
        return true;
    }
    edges(graph, node.key, Direction::In, &[EdgeKind::Calls])
        .iter()
        .any(|call| is_tested(graph, call.source))
}

/// Whether a test file the change touches covers `node`.
pub fn covered_by_touched_tests(
    graph: &dyn GraphQuery,
    touched_tests: &BTreeSet<&str>,
    node: &NodeRef<'_>,
) -> bool {
    let Some(symbol_file) = node.file else {
        return false;
    };
    for test_file in touched_tests {
        let mut covers = false;
        graph.nodes_in_file(test_file, &mut |test_node| {
            if covers {
                return;
            }
            if matches!(test_node.kind, NodeKind::TestSuite | NodeKind::TestCase)
                && test_node.name.contains(node.name)
            {
                covers = true;
                return;
            }
            for edge in edges(graph, test_node.key, Direction::Out, &[EdgeKind::Imports, EdgeKind::Tests]) {
                let target_file = graph.node(edge.target).and_then(|t| t.file);
                if target_file == Some(symbol_file) || edge.target == node.key {
                    covers = true;
                    return;
                }
            }
        });
        if covers {
            return true;
        }
    }
    false
}

fn finding(graph: &dyn GraphQuery, symbol: &ChangedSymbol, node: &NodeRef<'_>, handler: bool) -> DeterministicFinding {
    let callers = edges(graph, node.key, Direction::In, &[EdgeKind::Calls]).len();
    let severity_hint = if handler || callers > MANY_CALLERS {
        SeverityHint::Medium
    } else {
        SeverityHint::Low
    };
    let message = format!(
        "`{}` changed behavior but no test exercises it (no TESTS edge with confidence ≥ {MIN_TESTS_CONFIDENCE} within 1 hop, and no test changed in this PR covers it)",
        node.qualified_name
    );
    let evidence = deterministic(
        "precheck:tests_gap",
        EvidenceKind::TestBehavior,
        &message,
        location(&symbol.path, symbol.line, symbol.line),
        symbol_ref(node.id, node.qualified_name).into_iter().collect(),
    )
    .into_iter()
    .collect();
    DeterministicFinding {
        rule_id: RULE_ID.to_owned(),
        symbol_key: Some(node.id.to_owned()),
        path: symbol.path.clone(),
        line: symbol.line,
        message,
        severity_hint,
        details: vec![PrecheckEvidence::GraphAbsence {
            edge_kind: EdgeKind::Tests.as_str().to_owned(),
            min_confidence: MIN_TESTS_CONFIDENCE,
            searched_depth: 1,
        }],
        evidence,
        layers: None,
        basis_id: None,
    }
}

/// Runs the pre-check over the head graph. `test_framework_detected` comes from init facts;
/// without one the check reports `NOT_EXECUTED`.
pub fn tests_precheck(
    change: &ChangeSet,
    head: &dyn GraphQuery,
    test_framework_detected: bool,
) -> PrecheckOutcome {
    if !test_framework_detected {
        return PrecheckOutcome::not_executed("no test framework detected; test mapping unavailable");
    }
    let touched_tests: BTreeSet<&str> = change
        .files
        .iter()
        .filter(|f| f.status != FileStatus::Deleted && is_test_path(&f.path))
        .map(|f| f.path.as_str())
        .collect();
    let mut findings = Vec::new();
    for symbol in &change.symbols {
        if !symbol.behavioral || symbol.change == SymbolChange::Removed || is_test_path(&symbol.path) {
            continue;
        }
        let Some(node) = head.node(symbol.key) else {
            continue;
        };
        let handler = is_endpoint_handler(head, &node);
        if !(handler || is_public(head, &node)) {
            continue;
        }
        if has_test_coverage(head, &node) || covered_by_touched_tests(head, &touched_tests, &node) {
            continue;
        }
        findings.push(finding(head, symbol, &node, handler));
    }
    crate::metrics::deterministic_findings(RULE_ID, findings.len());
    PrecheckOutcome::executed(findings, false)
}
