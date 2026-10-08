//! Test suites and cases → `TestSuite`, `TestCase`, `TESTS` (CG-006).
//!
//! A test case points at the non-test symbols it calls inside its own range, and the confidence
//! of each `TESTS` edge is `derived(Framework, call_edge.confidence)`: a test that only reaches
//! its subject through an ambiguous name is a weaker claim than one that reaches it through an
//! import, and the graph has to say so.

use crate::edge_kind::EdgeKind;
use crate::node_id::{NodeId, NodeKey};
use crate::node_kind::NodeKind;

use super::contract::{self, FactIssueCode};
use super::Mapping;

/// `test_suite` → a `TestSuite` node contained by its file.
pub(crate) fn apply_suite(ctx: &mut Mapping<'_>) {
    let mut issues = Vec::new();
    let suite_path =
        contract::require_str(ctx.fact, ctx.category, ctx.path, "suite_path", &mut issues);
    let name = contract::require_str(ctx.fact, ctx.category, ctx.path, "name", &mut issues);
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    let (Some(suite_path), Some(name)) = (suite_path, name) else {
        return;
    };
    let Ok(id) = NodeId::test(ctx.path, &suite_path, &name) else {
        ctx.issue(
            FactIssueCode::InvalidAttribute,
            format!("test suite {name:?} does not produce a test id"),
        );
        return;
    };
    let suite = ctx.node(
        id.clone(),
        NodeKind::TestSuite,
        name.clone(),
        id.to_string(),
    );
    ctx.edge(
        EdgeKind::Contains,
        ctx.file.file_key,
        suite,
        crate::EdgeFlags::EMPTY,
    );
}

/// `test_case` → a `TestCase` node plus one `TESTS` edge per non-test symbol its body calls.
pub(crate) fn apply_case(ctx: &mut Mapping<'_>) {
    let mut issues = Vec::new();
    let suite_path =
        contract::require_str(ctx.fact, ctx.category, ctx.path, "suite_path", &mut issues);
    let name = contract::require_str(ctx.fact, ctx.category, ctx.path, "name", &mut issues);
    let range = contract::range_attr(ctx.fact.attrs.get("range"));
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    let (Some(suite_path), Some(name)) = (suite_path, name) else {
        return;
    };
    let Some(range) = range else {
        ctx.issue(
            FactIssueCode::MissingAttribute,
            "a test case needs a range attribute to know what it covers".to_owned(),
        );
        return;
    };
    let Ok(id) = NodeId::test(ctx.path, &suite_path, &name) else {
        ctx.issue(
            FactIssueCode::InvalidAttribute,
            format!("test case {name:?} does not produce a test id"),
        );
        return;
    };
    let parent = suite_key(ctx, &suite_path, &name);
    let case = ctx.node(id.clone(), NodeKind::TestCase, name.clone(), id.to_string());
    match parent {
        Some(suite) => ctx.edge(EdgeKind::Contains, suite, case, crate::EdgeFlags::EMPTY),
        None => ctx.edge(
            EdgeKind::Contains,
            ctx.file.file_key,
            case,
            crate::EdgeFlags::EMPTY,
        ),
    }

    // Every call this file resolved that lands inside the test's range, minus the test nodes.
    let mut covered: Vec<(NodeKey, crate::Confidence)> = Vec::new();
    for edge in ctx.file_edges {
        if edge.kind != EdgeKind::Calls {
            continue;
        }
        let Some(location) = &edge.location else {
            continue;
        };
        if location.file != *ctx.path {
            continue;
        }
        if !contains_line(range, location.line) {
            continue;
        }
        if is_test_node(ctx, &edge.target) {
            continue;
        }
        if !covered.iter().any(|(key, _)| *key == edge.target) {
            covered.push((edge.target, edge.confidence));
        }
    }
    covered.sort();
    for (target, confidence) in covered {
        ctx.derived_edge(EdgeKind::Tests, case, target, confidence, range.start.line);
    }
}

/// The `TestSuite` node of a case's suite, when the suite was mapped in the same file first.
fn suite_key(ctx: &Mapping<'_>, suite_path: &str, name: &str) -> Option<NodeKey> {
    let id = NodeId::test(ctx.path, suite_path, name).ok()?;
    ctx.out
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::TestSuite && node.id.as_str() == id.as_str())
        .map(|node| node.id.key())
}

/// True when the target is itself a test node, which must not appear as a `TESTS` subject.
fn is_test_node(ctx: &Mapping<'_>, key: &NodeKey) -> bool {
    ctx.file
        .symbols
        .iter()
        .find(|symbol| &symbol.key == key)
        .is_some_and(|symbol| symbol.flags.contains(crate::graph::NodeFlags::TEST))
        || ctx.out.nodes.iter().any(|node| {
            node.id.key() == *key
                && matches!(
                    node.kind,
                    NodeKind::TestCase | NodeKind::TestSuite | NodeKind::Fixture
                )
        })
}

/// Inclusive line containment: a call on the test's first or last line is inside it.
fn contains_line(range: review_core::location::SourceRange, line: u32) -> bool {
    line >= range.start.line && line <= range.end.line
}
