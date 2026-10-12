//! Type hierarchy relations (IMP-003).
//!
//! * A changed **member** `C.m`: the interface member `I.m` it implements (`Interface`), the
//!   supertype member it overrides (`Override`), subtype members overriding it
//!   (`OverriddenBy`), and sibling implementations `D.m` of the same `I.m` at distance 2
//!   (`Implementation`, path `C.m → I.m ← D.m`). When the linker emitted only the class-level
//!   `IMPLEMENTS`, the member-level hop is synthesized by name with confidence
//!   `class edge × 0.95`.
//! * A changed **interface member** `I.m`: every implementation at distance 1 — a signature
//!   change breaks the contract for all of them.
//! * A changed **type**: subtypes (reverse `EXTENDS`/`IMPLEMENTS`, depth ≤ 2), supertypes
//!   (forward, depth ≤ 2) and related types (reverse `USES_TYPE`/`ACCEPTS_TYPE`/`RETURNS_TYPE`,
//!   depth 1), so a changed DTO pulls in its handlers.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use codegraph::{Confidence, Direction, EdgeKind, EdgeKindSet, GraphQuery, NodeKey, NodeKind};

use super::builder::{collect_edges, Cx, SeedState};
use super::model::{GraphSide, Relation, TruncReason};
use super::path::{step, Extras, Trail};

/// Multiplier for a member-level hop synthesized from a class-level edge.
const SYNTHESIZED_FACTOR: f32 = 0.95;

fn is_member_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Method
            | NodeKind::Constructor
            | NodeKind::Handler
            | NodeKind::JobHandler
            | NodeKind::QueueProducer
            | NodeKind::Property
            | NodeKind::Field
            | NodeKind::Function
    )
}

fn is_type_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Struct
            | NodeKind::Trait
            | NodeKind::Enum
            | NodeKind::TypeAlias
            | NodeKind::Controller
            | NodeKind::DatabaseEntity
            | NodeKind::Middleware
            | NodeKind::QueueConsumer
    )
}

fn parent_of(graph: &dyn GraphQuery, node: NodeKey) -> Option<NodeKey> {
    graph.node(node).and_then(|view| view.attrs.parent)
}

fn is_interface(graph: &dyn GraphQuery, node: NodeKey) -> bool {
    graph
        .node(node)
        .is_some_and(|view| matches!(view.kind, NodeKind::Interface | NodeKind::Trait))
}

/// Members of `owner` named `name`.
fn members_named(graph: &dyn GraphQuery, owner: NodeKey, name: &str) -> Vec<NodeKey> {
    let mut out: Vec<NodeKey> = collect_edges(
        graph,
        owner,
        Direction::Out,
        EdgeKindSet::of(EdgeKind::Contains),
    )
    .into_iter()
    .filter(|edge| {
        graph
            .node(edge.target)
            .is_some_and(|view| view.name == name && is_member_kind(view.kind))
    })
    .map(|edge| edge.target)
    .collect();
    out.sort();
    out.dedup();
    out
}

fn scaled(confidence: Confidence) -> Confidence {
    Confidence::from_f32(confidence.as_f32() * SYNTHESIZED_FACTOR)
}

/// A member-level contract hop `member → target`.
#[derive(Debug, Clone, Copy)]
struct Hop {
    target: NodeKey,
    edge: EdgeKind,
    confidence: Confidence,
}

/// The supertype members `member` implements or overrides: linker edges first, then hops
/// synthesized from the owner's class-level `IMPLEMENTS`/`EXTENDS`.
fn contract_targets(graph: &dyn GraphQuery, member: NodeKey) -> Vec<Hop> {
    let kinds = EdgeKindSet::of(EdgeKind::Overrides).union(EdgeKindSet::of(EdgeKind::Implements));
    let mut out: Vec<Hop> = collect_edges(graph, member, Direction::Out, kinds)
        .into_iter()
        .filter(|edge| edge.target != member)
        .map(|edge| Hop {
            target: edge.target,
            edge: edge.kind,
            confidence: edge.confidence,
        })
        .collect();
    let known: BTreeSet<NodeKey> = out.iter().map(|hop| hop.target).collect();
    if let (Some(owner), Some(view)) = (parent_of(graph, member), graph.node(member)) {
        let name = view.name.to_owned();
        let class_kinds =
            EdgeKindSet::of(EdgeKind::Implements).union(EdgeKindSet::of(EdgeKind::Extends));
        for edge in collect_edges(graph, owner, Direction::Out, class_kinds) {
            for target in members_named(graph, edge.target, &name) {
                if target != member && !known.contains(&target) {
                    out.push(Hop {
                        target,
                        edge: EdgeKind::Implements,
                        confidence: scaled(edge.confidence),
                    });
                }
            }
        }
    }
    out.sort_by_key(|hop| (Reverse(hop.confidence), hop.target));
    out.dedup_by_key(|hop| hop.target);
    out
}

/// Members implementing or overriding `contract` (the reverse of [`contract_targets`]):
/// `(member, edge as stored from the member, confidence)`.
fn implementations_of(graph: &dyn GraphQuery, contract: NodeKey) -> Vec<Hop> {
    let kinds = EdgeKindSet::of(EdgeKind::Overrides).union(EdgeKindSet::of(EdgeKind::Implements));
    let mut out: Vec<Hop> = collect_edges(graph, contract, Direction::In, kinds)
        .into_iter()
        .filter(|edge| edge.source != contract)
        .map(|edge| Hop {
            target: edge.source,
            edge: edge.kind,
            confidence: edge.confidence,
        })
        .collect();
    let known: BTreeSet<NodeKey> = out.iter().map(|hop| hop.target).collect();
    if let (Some(owner), Some(view)) = (parent_of(graph, contract), graph.node(contract)) {
        let name = view.name.to_owned();
        let class_kinds =
            EdgeKindSet::of(EdgeKind::Implements).union(EdgeKindSet::of(EdgeKind::Extends));
        for edge in collect_edges(graph, owner, Direction::In, class_kinds) {
            for member in members_named(graph, edge.source, &name) {
                if member != contract && !known.contains(&member) {
                    out.push(Hop {
                        target: member,
                        edge: EdgeKind::Implements,
                        confidence: scaled(edge.confidence),
                    });
                }
            }
        }
    }
    out.sort_by_key(|hop| (Reverse(hop.confidence), hop.target));
    out.dedup_by_key(|hop| hop.target);
    out
}

/// Expands every type relation of the seed.
pub(crate) fn expand_types(cx: &Cx<'_>, state: &mut SeedState) {
    let Some(graph) = cx.graph(state.side) else {
        return;
    };
    let Some(kind) = graph.node(state.seed).map(|view| view.kind) else {
        return;
    };
    if is_type_kind(kind) {
        expand_type_seed(cx, graph, state);
    } else if is_member_kind(kind) {
        expand_member_seed(cx, graph, state);
    }
}

fn admit(
    cx: &Cx<'_>,
    state: &mut SeedState,
    relation: Relation,
    node: NodeKey,
    trail: Trail,
) -> bool {
    match cx.candidate(state.side, node, trail, Extras::default()) {
        Some(candidate) => state.admit(relation, cx.budget.max_type_relations, candidate),
        None => false,
    }
}

fn expand_member_seed(cx: &Cx<'_>, graph: &dyn GraphQuery, state: &mut SeedState) {
    let seed = state.seed;
    let side = state.side;
    let root = Trail::seed(seed);
    let seed_is_contract = parent_of(graph, seed).is_some_and(|owner| is_interface(graph, owner));

    // Upwards: the contracts the seed implements or overrides.
    let mut interfaces: Vec<(NodeKey, Trail)> = Vec::new();
    for hop in contract_targets(graph, seed) {
        let trail = root.extend(
            step(seed, hop.edge, hop.target, hop.confidence, side),
            hop.target,
            true,
        );
        let on_interface =
            parent_of(graph, hop.target).is_some_and(|owner| is_interface(graph, owner));
        let relation = if on_interface {
            Relation::Interface
        } else {
            Relation::Override
        };
        if admit(cx, state, relation, hop.target, trail.clone()) && on_interface {
            interfaces.push((hop.target, trail));
        }
    }

    // Downwards: members implementing or overriding the seed.
    for hop in implementations_of(graph, seed) {
        let trail = root.extend(
            step(hop.target, hop.edge, seed, hop.confidence, side),
            hop.target,
            true,
        );
        let relation = if seed_is_contract {
            Relation::Implementation
        } else {
            Relation::OverriddenBy
        };
        admit(cx, state, relation, hop.target, trail);
    }

    // Sideways: sibling implementations of the same interface member, at distance 2.
    for (interface, trail) in interfaces {
        for hop in implementations_of(graph, interface) {
            if hop.target == seed || trail.visits(hop.target) {
                continue;
            }
            let sibling = trail.extend(
                step(hop.target, hop.edge, interface, hop.confidence, side),
                hop.target,
                true,
            );
            admit(cx, state, Relation::Implementation, hop.target, sibling);
        }
    }
}

/// Bounded BFS over `kinds` in `dir`, emitting every reached node under `relation`.
fn hierarchy_bfs(
    cx: &Cx<'_>,
    graph: &dyn GraphQuery,
    state: &mut SeedState,
    dir: Direction,
    kinds: EdgeKindSet,
    max_depth: u8,
    relation: Relation,
) {
    let side: GraphSide = state.side;
    let mut level: Vec<(NodeKey, Trail)> = vec![(state.seed, Trail::seed(state.seed))];
    let mut seen: BTreeSet<NodeKey> = BTreeSet::new();
    seen.insert(state.seed);
    for depth in 1..=max_depth {
        let mut candidates: Vec<(Confidence, NodeKey, Trail)> = Vec::new();
        for (node, trail) in &level {
            for edge in collect_edges(graph, *node, dir, kinds) {
                let other = if dir == Direction::In {
                    edge.source
                } else {
                    edge.target
                };
                if seen.contains(&other) {
                    continue;
                }
                let next = trail.extend(
                    step(edge.source, edge.kind, edge.target, edge.confidence, side),
                    other,
                    true,
                );
                candidates.push((edge.confidence, other, next));
            }
        }
        candidates.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| super::path::compare_trails(&a.2, &b.2))
        });
        let mut next_level: Vec<(NodeKey, Trail)> = Vec::new();
        for (_, node, trail) in candidates {
            let strong = trail.min_confidence >= cx.budget.min_confidence;
            if admit(cx, state, relation, node, trail.clone()) && seen.insert(node) && strong {
                next_level.push((node, trail));
            }
        }
        if depth == max_depth && !next_level.is_empty() {
            let beyond: BTreeSet<NodeKey> = next_level
                .iter()
                .flat_map(|(node, _)| collect_edges(graph, *node, dir, kinds))
                .map(|edge| {
                    if dir == Direction::In {
                        edge.source
                    } else {
                        edge.target
                    }
                })
                .filter(|other| !seen.contains(other))
                .collect();
            if !beyond.is_empty() {
                state.record(
                    Some(relation),
                    TruncReason::Depth,
                    u32::from(max_depth),
                    beyond.len() as u32,
                    None,
                );
            }
        }
        level = next_level;
        if level.is_empty() {
            break;
        }
    }
}

fn expand_type_seed(cx: &Cx<'_>, graph: &dyn GraphQuery, state: &mut SeedState) {
    let hierarchy = EdgeKindSet::of(EdgeKind::Extends).union(EdgeKindSet::of(EdgeKind::Implements));
    hierarchy_bfs(
        cx,
        graph,
        state,
        Direction::In,
        hierarchy,
        2,
        Relation::Subtype,
    );
    hierarchy_bfs(
        cx,
        graph,
        state,
        Direction::Out,
        hierarchy,
        2,
        Relation::Supertype,
    );
    let uses = EdgeKindSet::from_kinds([
        EdgeKind::UsesType,
        EdgeKind::AcceptsType,
        EdgeKind::ReturnsType,
    ]);
    hierarchy_bfs(
        cx,
        graph,
        state,
        Direction::In,
        uses,
        1,
        Relation::RelatedType,
    );
}
