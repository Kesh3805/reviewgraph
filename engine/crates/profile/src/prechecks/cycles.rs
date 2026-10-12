//! New module-level import cycles (REV-A-002, part 2).
//!
//! The module graph is a projection of `IMPORTS` edges onto modules, built lazily from the
//! modules the pull request touches (forward reachability, at most [`MAX_MODULES`] modules).
//! Strongly connected components come from Tarjan's algorithm on head and base; a cycle is
//! reported only when its sorted member set is absent from the base (`new = scc_head − scc_base`).
//! Symbol-level cycles (recursion) and whole-repository reports are out of scope.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::ControlFlow;
use std::time::Instant;

use codegraph::{Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, NodeKey};
use review_core::evidence::EvidenceKind;

use super::boundaries::new_edges;
use super::{DeterministicFinding, PrecheckEvidence, PrecheckOutcome, SeverityHint};
use crate::infer::architecture::ModuleIndex;
use crate::policy::change::{ChangeSet, GraphPair};
use crate::policy::evidence::{deterministic, location};

pub const RULE_ID: &str = "architecture.new_cycle";
/// Projection budget.
pub const MAX_MODULES: usize = 2_000;

/// Module → nodes of a graph.
fn nodes_by_module(graph: &dyn GraphQuery, modules: &ModuleIndex) -> BTreeMap<String, Vec<NodeKey>> {
    let mut out: BTreeMap<String, Vec<NodeKey>> = BTreeMap::new();
    graph.for_each_node(&mut |node| {
        if let Some(path) = node.file {
            out.entry(modules.module_of(path)).or_default().push(node.key);
        }
    });
    out
}

/// The module import graph reachable from `start`, and whether the budget cut it short.
fn module_graph(
    graph: &dyn GraphQuery,
    modules: &ModuleIndex,
    start: &BTreeSet<String>,
) -> (BTreeMap<String, BTreeSet<String>>, bool) {
    let members = nodes_by_module(graph, modules);
    let filter = EdgeFilter::kinds(EdgeKindSet::of(EdgeKind::Imports));
    let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut seen: BTreeSet<String> = start.clone();
    let mut queue: VecDeque<String> = start.iter().cloned().collect();
    let mut truncated = false;
    while let Some(module) = queue.pop_front() {
        let mut targets = BTreeSet::new();
        for key in members.get(&module).map(Vec::as_slice).unwrap_or_default() {
            graph.for_each_edge(*key, Direction::Out, &filter, &mut |edge| {
                if let Some(path) = graph.node(edge.target).and_then(|n| n.file) {
                    let target = modules.module_of(path);
                    if target != module {
                        targets.insert(target);
                    }
                }
                ControlFlow::Continue(())
            });
        }
        for target in &targets {
            if !seen.contains(target) {
                if seen.len() >= MAX_MODULES {
                    truncated = true;
                    continue;
                }
                seen.insert(target.clone());
                queue.push_back(target.clone());
            }
        }
        adjacency.insert(module, targets);
    }
    (adjacency, truncated)
}

/// Tarjan's strongly connected components; each component sorted, the list sorted.
pub fn tarjan(adjacency: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    struct State<'a> {
        adjacency: &'a BTreeMap<String, BTreeSet<String>>,
        index: BTreeMap<&'a str, usize>,
        low: BTreeMap<&'a str, usize>,
        stack: Vec<&'a str>,
        on_stack: BTreeSet<&'a str>,
        next: usize,
        out: Vec<Vec<String>>,
    }

    fn visit<'a>(s: &mut State<'a>, v: &'a str) {
        s.index.insert(v, s.next);
        s.low.insert(v, s.next);
        s.next += 1;
        s.stack.push(v);
        s.on_stack.insert(v);
        let adjacency = s.adjacency;
        if let Some(targets) = adjacency.get(v) {
            for w in targets {
                let w = w.as_str();
                if !s.index.contains_key(w) {
                    visit(s, w);
                    let low_w = s.low.get(w).copied().unwrap_or(usize::MAX);
                    if let Some(low_v) = s.low.get_mut(v) {
                        *low_v = (*low_v).min(low_w);
                    }
                } else if s.on_stack.contains(w) {
                    let index_w = s.index.get(w).copied().unwrap_or(usize::MAX);
                    if let Some(low_v) = s.low.get_mut(v) {
                        *low_v = (*low_v).min(index_w);
                    }
                }
            }
        }
        if s.low.get(v) == s.index.get(v) {
            let mut component = Vec::new();
            while let Some(w) = s.stack.pop() {
                s.on_stack.remove(w);
                component.push(w.to_owned());
                if w == v {
                    break;
                }
            }
            component.sort();
            s.out.push(component);
        }
    }

    let mut state = State {
        adjacency,
        index: BTreeMap::new(),
        low: BTreeMap::new(),
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in adjacency.keys() {
        if !state.index.contains_key(v.as_str()) {
            visit(&mut state, v.as_str());
        }
    }
    let mut out = state.out;
    out.sort();
    out
}

/// Cycles (components with more than one module) that contain a touched module.
fn cycles(
    graph: &dyn GraphQuery,
    modules: &ModuleIndex,
    touched: &BTreeSet<String>,
) -> (BTreeSet<Vec<String>>, bool) {
    let (adjacency, truncated) = module_graph(graph, modules, touched);
    let found = tarjan(&adjacency)
        .into_iter()
        .filter(|c| c.len() > 1 && c.iter().any(|m| touched.contains(m)))
        .collect();
    (found, truncated)
}

/// Reports every import cycle among modules that the change creates.
pub fn detect_new_cycles(
    change: &ChangeSet,
    graphs: &GraphPair<'_>,
    modules: &ModuleIndex,
) -> PrecheckOutcome {
    let started = Instant::now();
    let touched: BTreeSet<String> = change.live_files().map(|p| modules.module_of(p)).collect();
    if touched.is_empty() {
        return PrecheckOutcome::executed(Vec::new(), false);
    }
    let (head, head_truncated) = cycles(graphs.head, modules, &touched);
    let (base, base_truncated) = cycles(graphs.base, modules, &touched);
    let added = new_edges(
        change,
        graphs.head,
        graphs.base,
        EdgeKindSet::of(EdgeKind::Imports),
    );
    let mut findings = Vec::new();
    for members in head.difference(&base) {
        let in_cycle: BTreeSet<&str> = members.iter().map(String::as_str).collect();
        let anchor = added.iter().find(|e| {
            let from = e.source.file.map(|f| modules.module_of(f));
            let to = e.target.file.map(|f| modules.module_of(f));
            matches!((from, to), (Some(a), Some(b)) if a != b && in_cycle.contains(a.as_str()) && in_cycle.contains(b.as_str()))
        });
        let (path, line, symbol) = match anchor {
            Some(e) => (e.path.to_owned(), e.line, Some(e.source.id.to_owned())),
            None => (
                change.live_files().next().unwrap_or_default().to_owned(),
                1,
                None,
            ),
        };
        let message = format!(
            "this change creates an import cycle between modules {}",
            members.join(" → ")
        );
        let evidence = deterministic(
            "precheck:cycles",
            EvidenceKind::StaticAnalysisResult,
            &message,
            location(&path, line, line),
            Vec::new(),
        )
        .into_iter()
        .collect();
        findings.push(DeterministicFinding {
            rule_id: RULE_ID.to_owned(),
            symbol_key: symbol,
            path,
            line,
            message,
            severity_hint: SeverityHint::Medium,
            details: vec![PrecheckEvidence::Cycle {
                members: members.clone(),
            }],
            evidence,
            layers: None,
            basis_id: None,
        });
    }
    crate::metrics::cycle_detection_duration(started.elapsed().as_secs_f64());
    crate::metrics::deterministic_findings(RULE_ID, findings.len());
    PrecheckOutcome::executed(findings, head_truncated || base_truncated)
}
