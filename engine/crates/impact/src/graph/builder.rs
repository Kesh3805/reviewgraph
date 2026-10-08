//! Orchestrates the per-seed expansions into an [`ImpactGraph`] (IMP-002..IMP-007).
//!
//! # Determinism under parallelism
//!
//! Seeds are expanded in parallel (rayon) for every relation at depth ≤ 2. The per-PR element
//! cap is *not* consulted in that pass: if `seeds × max_total_elements_per_symbol` would exceed
//! `max_total_elements_pr`, every seed's cap is lowered up front to `floor(pr_cap / seeds)`, so
//! the sum is bounded without any shared counter and the result cannot depend on thread timing.
//! The depth-3 caller pass then runs sequentially, in descending seed priority, against the PR
//! budget that remains — and only when at least half of it does.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::time::Instant;

use codegraph::{
    Confidence, Direction, EdgeFilter, EdgeKind, EdgeKindSet, GraphQuery, GraphQueryExt, NodeKey,
};
use rayon::prelude::*;
use review_core::ids::SymbolKey;

use crate::input::{ChangeSet, SymbolInput};
use crate::metrics;

use super::budget::ImpactBudget;
use super::calls;
use super::entrypoints;
use super::model::{
    compute_input_hash, GraphSide, ImpactFlags, ImpactGraph, ImpactStats, Relation, SeedSkip,
    SeedTruncation, SymbolImpact, TruncReason, Truncation, IMPACT_SCHEMA_VERSION,
};
use super::path::{Candidate, ElementSet, Extras, Offer, Trail};
use super::types;

/// An owned copy of the parts of an edge the expansions need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EdgeView {
    pub kind: EdgeKind,
    pub source: NodeKey,
    pub target: NodeKey,
    pub confidence: Confidence,
}

/// Every edge of `key` in `dir` whose kind is in `kinds`, in the graph's canonical order.
pub(crate) fn collect_edges(
    graph: &dyn GraphQuery,
    key: NodeKey,
    dir: Direction,
    kinds: EdgeKindSet,
) -> Vec<EdgeView> {
    let mut out = Vec::new();
    graph.for_each_edge(key, dir, &EdgeFilter::kinds(kinds), &mut |edge| {
        out.push(EdgeView {
            kind: edge.kind,
            source: edge.source,
            target: edge.target,
            confidence: edge.confidence,
        });
        ControlFlow::Continue(())
    });
    out
}

/// Inputs of one impact build.
#[derive(Clone, Copy)]
pub struct ImpactInputs<'a> {
    pub change: &'a ChangeSet,
    pub head: &'a dyn GraphQuery,
    /// Required for removed callees and the impact of removed symbols.
    pub base: Option<&'a dyn GraphQuery>,
    pub budget: &'a ImpactBudget,
    /// Seed priority for the depth-3 pass (RISK-004 score); change-class count when absent.
    pub priority: Option<&'a BTreeMap<SymbolKey, f32>>,
    /// Expand seeds on the rayon pool. The result is identical either way.
    pub parallel: bool,
}

impl std::fmt::Debug for ImpactInputs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImpactInputs")
            .field("symbols", &self.change.symbols.len())
            .field("base", &self.base.is_some())
            .field("budget", self.budget)
            .field("parallel", &self.parallel)
            .finish()
    }
}

/// Shared, read-only context of every seed expansion.
pub(crate) struct Cx<'a> {
    pub head: &'a dyn GraphQuery,
    pub base: Option<&'a dyn GraphQuery>,
    pub budget: &'a ImpactBudget,
    pub change: &'a ChangeSet,
}

impl<'a> Cx<'a> {
    pub fn graph(&self, side: GraphSide) -> Option<&'a dyn GraphQuery> {
        match side {
            GraphSide::Head => Some(self.head),
            GraphSide::Base => self.base,
        }
    }

    /// A candidate for `node` on `side`, or `None` when the node is not in that graph.
    pub fn candidate(
        &self,
        side: GraphSide,
        node: NodeKey,
        trail: Trail,
        extras: Extras,
    ) -> Option<Candidate> {
        let graph = self.graph(side)?;
        let view = graph.node(node)?;
        Some(Candidate {
            node,
            node_id: view.id.to_owned(),
            kind: view.kind,
            trail,
            extras,
        })
    }
}

/// A truncation being accumulated: `(relation, reason)` → `(limit, dropped, visited)`.
type DropKey = (Option<Relation>, TruncReason);

/// The mutable state of one seed's expansion.
#[derive(Debug)]
pub(crate) struct SeedState {
    pub seed: NodeKey,
    pub side: GraphSide,
    pub set: ElementSet,
    pub visited: BTreeSet<NodeKey>,
    counts: BTreeMap<Relation, u32>,
    cap: u32,
    remaining: u32,
    /// The PR-wide remainder, consulted only in the sequential depth-3 pass.
    pub pr_remaining: Option<u32>,
    pr_cap: u32,
    drops: BTreeMap<DropKey, (u32, u32, Option<u32>)>,
    /// Caller nodes at the deepest level reached, for the depth-3 pass.
    pub caller_frontier: Vec<NodeKey>,
    /// Nodes the endpoint search visited.
    pub endpoint_visits: u32,
}

impl SeedState {
    pub fn new(seed: NodeKey, side: GraphSide, cap: u32, pr_cap: u32) -> Self {
        let mut visited = BTreeSet::new();
        visited.insert(seed);
        Self {
            seed,
            side,
            set: ElementSet::new(),
            visited,
            counts: BTreeMap::new(),
            cap,
            remaining: cap,
            pr_remaining: None,
            pr_cap,
            drops: BTreeMap::new(),
            caller_frontier: Vec::new(),
            endpoint_visits: 0,
        }
    }

    /// Elements held for `relation`.
    pub fn count(&self, relation: Relation) -> u32 {
        self.counts.get(&relation).copied().unwrap_or(0)
    }

    /// Offers a candidate, enforcing the relation's `limit`, the seed's total cap and (in the
    /// transitive pass) the PR cap. Returns whether the element is held after the call.
    pub fn admit(&mut self, relation: Relation, limit: u32, candidate: Candidate) -> bool {
        if self.set.contains(relation, candidate.node) {
            self.set.offer(relation, candidate);
            return true;
        }
        if self.count(relation) >= limit {
            self.record(Some(relation), TruncReason::Limit, limit, 1, None);
            return false;
        }
        if self.remaining == 0 {
            let cap = self.cap;
            self.record(Some(relation), TruncReason::TotalCap, cap, 1, None);
            return false;
        }
        if let Some(pr_remaining) = self.pr_remaining {
            if pr_remaining == 0 {
                let cap = self.pr_cap;
                self.record(Some(relation), TruncReason::TotalCap, cap, 1, None);
                return false;
            }
            self.pr_remaining = Some(pr_remaining - 1);
        }
        if self.set.offer(relation, candidate) == Offer::Inserted {
            self.remaining -= 1;
            *self.counts.entry(relation).or_insert(0) += 1;
        }
        true
    }

    /// Accumulates a truncation; repeated stops of the same kind add up.
    pub fn record(
        &mut self,
        relation: Option<Relation>,
        reason: TruncReason,
        limit: u32,
        dropped: u32,
        visited: Option<u32>,
    ) {
        let entry = self
            .drops
            .entry((relation, reason))
            .or_insert((limit, 0, None));
        entry.0 = limit;
        entry.1 = entry.1.saturating_add(dropped);
        if visited.is_some() {
            entry.2 = visited;
        }
    }

    /// Forgets a truncation that a later pass made untrue (the depth-3 pass explores the
    /// frontier the depth-2 pass reported).
    pub fn clear(&mut self, relation: Option<Relation>, reason: TruncReason) {
        self.drops.remove(&(relation, reason));
    }

    fn truncations(&self) -> Vec<Truncation> {
        self.drops
            .iter()
            .map(
                |((relation, reason), (limit, dropped, visited))| Truncation {
                    relation: *relation,
                    limit: *limit,
                    dropped: *dropped,
                    reason: *reason,
                    visited: *visited,
                },
            )
            .collect()
    }

    fn used(&self) -> u32 {
        self.cap - self.remaining
    }
}

/// The finished expansion of one seed, before it is frozen.
#[derive(Debug)]
struct Expanded {
    input: usize,
    state: Option<SeedState>,
    skipped: Option<SeedSkip>,
    missing: bool,
}

/// The order seeds are reported in: the change model's `(path, range start, symbol id)`.
fn seed_order(change: &ChangeSet) -> Vec<usize> {
    let mut order: Vec<usize> = (0..change.symbols.len()).collect();
    order.sort_by(|a, b| {
        let (a, b) = (&change.symbols[*a], &change.symbols[*b]);
        a.path()
            .cmp(b.path())
            .then_with(|| a.symbol.range.start.cmp(&b.symbol.range.start))
            .then_with(|| a.id().cmp(b.id()))
    });
    order.dedup_by_key(|index| change.symbols[*index].key());
    order
}

/// Pass 1 for one seed: every relation at depth ≤ 2 (callers up to `min(depth, 2)`).
fn expand_seed(cx: &Cx<'_>, index: usize, symbol: &SymbolInput, cap: u32) -> Expanded {
    if symbol.generated {
        return Expanded {
            input: index,
            state: None,
            skipped: Some(SeedSkip::Generated),
            missing: false,
        };
    }
    if symbol.cosmetic {
        return Expanded {
            input: index,
            state: None,
            skipped: Some(SeedSkip::Cosmetic),
            missing: false,
        };
    }
    let side = if symbol.is_removed() {
        GraphSide::Base
    } else {
        GraphSide::Head
    };
    let seed = symbol.key();
    let present = cx
        .graph(side)
        .is_some_and(|graph| graph.node(seed).is_some());
    let mut state = SeedState::new(seed, side, cap, cx.budget.max_total_elements_pr);
    if !present {
        state.record(None, TruncReason::SeedMissing, 0, 0, None);
        return Expanded {
            input: index,
            state: Some(state),
            skipped: None,
            missing: true,
        };
    }

    let first_pass_depth = cx.budget.max_caller_depth.min(2);
    state.caller_frontier = calls::expand_callers(cx, &mut state, vec![seed], 0, first_pass_depth);
    calls::expand_callees(cx, &mut state);
    let removed: Vec<NodeKey> = symbol
        .removed_calls
        .iter()
        .filter_map(|call| call.target_key())
        .collect();
    calls::expand_removed_callees(cx, &mut state, &removed);
    types::expand_types(cx, &mut state);
    state.endpoint_visits = entrypoints::expand_endpoints(cx, &mut state);

    Expanded {
        input: index,
        state: Some(state),
        skipped: None,
        missing: false,
    }
}

/// Resource node ids a caller itself touches (tables, queues), at most five.
fn caller_touches(graph: &dyn GraphQuery, caller: NodeKey) -> Vec<String> {
    let kinds = EdgeKindSet::from_kinds([
        EdgeKind::ReadsTable,
        EdgeKind::WritesTable,
        EdgeKind::ProducesJob,
        EdgeKind::Publishes,
    ]);
    let mut ids: Vec<String> = graph
        .out_edges(caller, kinds)
        .iter()
        .filter_map(|edge| graph.node(edge.target).map(|node| node.id.to_owned()))
        .collect();
    ids.sort();
    ids.dedup();
    ids.truncate(5);
    ids
}

/// Builds the impact graph of every changed symbol.
pub fn build_impact(inputs: &ImpactInputs<'_>) -> ImpactGraph {
    let started = Instant::now();
    let change = inputs.change;
    let budget = inputs.budget;
    let span = tracing::info_span!(
        "impact_analysis",
        seeds = change.symbols.len() as u64,
        elements = tracing::field::Empty,
        truncated_relations = tracing::field::Empty,
        transitive_pass_ran = tracing::field::Empty,
        endpoints_found = tracing::field::Empty,
    );
    let _entered = span.enter();
    let cx = Cx {
        head: inputs.head,
        base: inputs.base,
        budget,
        change,
    };

    let order = seed_order(change);
    let expandable = order
        .iter()
        .filter(|index| !change.symbols[**index].is_skipped())
        .count() as u32;
    let pr_cap = budget.max_total_elements_pr;
    let mut per_seed_cap = budget.max_total_elements_per_symbol;
    if expandable > 0 && u64::from(expandable) * u64::from(per_seed_cap) > u64::from(pr_cap) {
        per_seed_cap = pr_cap / expandable;
    }

    // Pass 1: depth ≤ 2, independent per seed.
    let mut expanded: Vec<Expanded> = if inputs.parallel {
        order
            .par_iter()
            .map(|index| expand_seed(&cx, *index, &change.symbols[*index], per_seed_cap))
            .collect()
    } else {
        order
            .iter()
            .map(|index| expand_seed(&cx, *index, &change.symbols[*index], per_seed_cap))
            .collect()
    };

    // Pass 2: depth 3, sequential in descending priority, only with half the PR budget left.
    let used: u32 = expanded
        .iter()
        .filter_map(|e| e.state.as_ref().map(SeedState::used))
        .sum();
    let mut pr_remaining = pr_cap.saturating_sub(used);
    let mut transitive_pass_ran = false;
    if budget.max_caller_depth >= 3 && u64::from(pr_remaining) * 2 >= u64::from(pr_cap) {
        transitive_pass_ran = true;
        let mut ranked: Vec<usize> = (0..expanded.len()).collect();
        let priority = |position: usize| -> f32 {
            let symbol = &change.symbols[expanded[position].input];
            match inputs.priority.and_then(|p| p.get(&symbol.key())) {
                Some(score) => *score,
                None => symbol.classes.len() as f32,
            }
        };
        ranked.sort_by(|a, b| priority(*b).total_cmp(&priority(*a)).then_with(|| a.cmp(b)));
        for position in ranked {
            if expanded[position].missing {
                continue;
            }
            let Some(state) = expanded[position].state.as_mut() else {
                continue;
            };
            if state.caller_frontier.is_empty() {
                continue;
            }
            state.pr_remaining = Some(pr_remaining);
            state.clear(Some(Relation::Caller), TruncReason::Depth);
            let frontier = std::mem::take(&mut state.caller_frontier);
            let start = budget.max_caller_depth.min(2);
            state.caller_frontier =
                calls::expand_callers(&cx, state, frontier, start, budget.max_caller_depth);
            pr_remaining = state.pr_remaining.unwrap_or(pr_remaining);
            state.pr_remaining = None;
        }
    }

    let pr_exhausted = pr_remaining == 0;

    // Freeze.
    let mut stats = ImpactStats {
        transitive_pass_ran,
        ..ImpactStats::default()
    };
    let mut symbols = Vec::with_capacity(expanded.len());
    for item in expanded {
        let symbol = &change.symbols[item.input];
        let seed = symbol.key();
        stats.seeds += 1;
        let Some(mut state) = item.state else {
            symbols.push(SymbolImpact {
                seed,
                seed_id: symbol.id().to_owned(),
                side: GraphSide::Head,
                skipped: item.skipped,
                untested: false,
                elements: Vec::new(),
                truncation: Vec::new(),
            });
            continue;
        };
        if item.missing {
            stats.seeds_without_graph += 1;
        }
        stats.endpoint_search_visits += state.endpoint_visits;
        if let Some(graph) = cx.graph(state.side) {
            let callers: Vec<NodeKey> = state
                .set
                .of(Relation::Caller)
                .into_iter()
                .map(|(node, _)| node)
                .collect();
            for caller in callers {
                let touches = caller_touches(graph, caller);
                let missing_on_head =
                    state.side == GraphSide::Base && inputs.head.node(caller).is_none();
                if let Some(extras) = state.set.extras_mut(Relation::Caller, caller) {
                    extras.touches = touches;
                    extras.missing_on_head = missing_on_head;
                }
            }
        }
        let truncation = state.truncations();
        let side = state.side;
        let elements = std::mem::take(&mut state.set).into_elements(budget);
        for element in &elements {
            *stats
                .elements_by_relation
                .entry(element.relation)
                .or_insert(0) += 1;
            if element.weak {
                stats.weak_elements += 1;
            }
        }
        stats.total_elements += elements.len() as u32;
        for t in &truncation {
            stats.truncations.push(SeedTruncation {
                seed,
                truncation: t.clone(),
            });
        }
        symbols.push(SymbolImpact {
            seed,
            seed_id: symbol.id().to_owned(),
            side,
            skipped: None,
            untested: false,
            elements,
            truncation,
        });
    }

    let flags = ImpactFlags {
        test_mapping_degraded: false,
        resource_facts_available: false,
        base_graph_missing: inputs.base.is_none(),
    };
    let graph = ImpactGraph {
        schema_version: IMPACT_SCHEMA_VERSION,
        input_hash: compute_input_hash(
            &change.input_hash,
            &change.head_snapshot,
            &change.base_snapshot,
            budget,
        ),
        symbols,
        budget: budget.clone(),
        stats,
        flags,
        test_targets: Vec::new(),
    };
    span.record("elements", u64::from(graph.stats.total_elements));
    span.record("truncated_relations", graph.stats.truncations.len() as u64);
    span.record("transitive_pass_ran", graph.stats.transitive_pass_ran);
    span.record(
        "endpoints_found",
        graph
            .stats
            .elements_by_relation
            .get(&Relation::Endpoint)
            .copied()
            .map_or(0, u64::from),
    );
    if pr_exhausted {
        metrics::record_budget_exhausted();
    }
    metrics::record_impact(&graph, started.elapsed());
    graph
}
