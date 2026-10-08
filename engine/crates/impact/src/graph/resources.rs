//! Config, database, queue and external API relations (IMP-006).
//!
//! * Distance 1 — the seed's own resource edges: `READS_CONFIG`/`WRITES_CONFIG` (config or env
//!   var), `READS_TABLE`/`WRITES_TABLE`, `PRODUCES_JOB`/`PUBLISHES`, `CONSUMES_JOB`/`SUBSCRIBES`,
//!   `DEPENDS_ON` an external API or dependency; plus the ORM entities injected into the seed's
//!   container class (`Repository<User>` → `User`).
//! * Distance 2 — the other side: consumers of a queue the seed produces to, other writers and
//!   readers of a table it writes, other readers of an env var it reads — each capped at
//!   `max_resource_other_side`.
//! * Removed edges — resource edges present on base only are emitted from the base graph with
//!   `removed = true`.
//!
//! Only the seed's own resources are seed relations: a caller's table is recorded on the caller
//! element (`touches`), never duplicated here. Env vars are names only; values never exist in
//! the graph.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use codegraph::{Direction, EdgeKind, EdgeKindSet, GraphQuery, NodeKey, NodeKind};

use super::builder::{collect_edges, Cx, EdgeView, SeedState};
use super::model::{GraphSide, Relation, ResourceAttrs, ResourceRole, TruncReason};
use super::path::{step, Extras, Trail};

const RESOURCE_KINDS: [EdgeKind; 9] = [
    EdgeKind::ReadsConfig,
    EdgeKind::WritesConfig,
    EdgeKind::ReadsTable,
    EdgeKind::WritesTable,
    EdgeKind::ProducesJob,
    EdgeKind::ConsumesJob,
    EdgeKind::Publishes,
    EdgeKind::Subscribes,
    EdgeKind::DependsOn,
];

/// Does the graph hold any resource node at all (are the framework adapters present)?
pub(crate) fn facts_available(graph: &dyn GraphQuery) -> bool {
    let mut found = false;
    graph.for_each_node(&mut |node| {
        if !found
            && matches!(
                node.kind,
                NodeKind::DatabaseTable
                    | NodeKind::DatabaseEntity
                    | NodeKind::Queue
                    | NodeKind::EnvironmentVariable
                    | NodeKind::Configuration
                    | NodeKind::ExternalApi
                    | NodeKind::ExternalDependency
            )
        {
            found = true;
        }
    });
    found
}

/// The relation and role of a seed's resource edge, or `None` for an edge that is not a
/// resource relation (a `DEPENDS_ON` onto an internal module).
fn classify(edge: EdgeKind, target: NodeKind) -> Option<(Relation, ResourceRole)> {
    let config_relation = if target == NodeKind::EnvironmentVariable {
        Relation::EnvVar
    } else {
        Relation::Config
    };
    match edge {
        EdgeKind::ReadsConfig => Some((config_relation, ResourceRole::Reads)),
        EdgeKind::WritesConfig => Some((config_relation, ResourceRole::Writes)),
        EdgeKind::ReadsTable => Some((Relation::DbTable, ResourceRole::Reads)),
        EdgeKind::WritesTable => Some((Relation::DbTable, ResourceRole::Writes)),
        EdgeKind::ProducesJob => Some((Relation::QueueProducer, ResourceRole::Produces)),
        EdgeKind::Publishes => Some((Relation::QueueProducer, ResourceRole::Publishes)),
        EdgeKind::ConsumesJob => Some((Relation::QueueConsumer, ResourceRole::Consumes)),
        EdgeKind::Subscribes => Some((Relation::QueueConsumer, ResourceRole::Subscribes)),
        EdgeKind::DependsOn
            if matches!(target, NodeKind::ExternalApi | NodeKind::ExternalDependency) =>
        {
            Some((Relation::ExternalApi, ResourceRole::DependsOn))
        }
        _ => None,
    }
}

fn resource_extras(role: ResourceRole, removed: bool) -> Extras {
    Extras {
        resource: Some(ResourceAttrs { role, removed }),
        ..Extras::default()
    }
}

/// The seed's resource edges on `graph`, strongest first.
fn resource_edges(graph: &dyn GraphQuery, seed: NodeKey) -> Vec<(EdgeView, NodeKind)> {
    let mut edges: Vec<(EdgeView, NodeKind)> = collect_edges(
        graph,
        seed,
        Direction::Out,
        EdgeKindSet::from_kinds(RESOURCE_KINDS),
    )
    .into_iter()
    .filter_map(|edge| graph.node(edge.target).map(|view| (edge, view.kind)))
    .collect();
    edges.sort_by_key(|(edge, _)| (Reverse(edge.confidence), edge.kind, edge.target));
    edges
}

/// Emits the other side of one resource: sources of `kinds` edges onto `resource`, other than
/// the seed, as `relation` with `role`, at most `max_resource_other_side` of them.
#[allow(clippy::too_many_arguments)]
fn other_side(
    cx: &Cx<'_>,
    graph: &dyn GraphQuery,
    state: &mut SeedState,
    resource: NodeKey,
    via: &Trail,
    kinds: EdgeKindSet,
    relation: Relation,
    role: ResourceRole,
    emitted: &mut u32,
) {
    let side = state.side;
    let mut sources: Vec<EdgeView> = collect_edges(graph, resource, Direction::In, kinds)
        .into_iter()
        .filter(|edge| edge.source != state.seed)
        .collect();
    sources.sort_by_key(|edge| (Reverse(edge.confidence), edge.source));
    let cap = cx.budget.max_resource_other_side;
    let mut dropped = 0u32;
    for edge in sources {
        if via.visits(edge.source) {
            continue;
        }
        if *emitted >= cap {
            dropped += 1;
            continue;
        }
        let trail = via.extend(
            step(edge.source, edge.kind, resource, edge.confidence, side),
            edge.source,
            true,
        );
        if let Some(candidate) =
            cx.candidate(side, edge.source, trail, resource_extras(role, false))
        {
            if state.admit(relation, cx.budget.max_resources, candidate) {
                *emitted += 1;
            }
        }
    }
    if dropped > 0 {
        state.record(Some(relation), TruncReason::Limit, cap, dropped, None);
    }
}

/// Expands every resource relation of the seed.
pub(crate) fn expand_resources(cx: &Cx<'_>, state: &mut SeedState) {
    let Some(graph) = cx.graph(state.side) else {
        return;
    };
    let seed = state.seed;
    let side = state.side;
    let root = Trail::seed(seed);
    let limit = cx.budget.max_resources;

    let own = resource_edges(graph, seed);
    let own_identity: BTreeSet<(EdgeKind, NodeKey)> = own
        .iter()
        .map(|(edge, _)| (edge.kind, edge.target))
        .collect();

    // Distance 1: the seed's own resources.
    let mut other_sides: Vec<(NodeKey, EdgeKind, Trail)> = Vec::new();
    for (edge, target_kind) in &own {
        let Some((relation, role)) = classify(edge.kind, *target_kind) else {
            continue;
        };
        let trail = root.extend(
            step(seed, edge.kind, edge.target, edge.confidence, side),
            edge.target,
            true,
        );
        let Some(candidate) = cx.candidate(
            side,
            edge.target,
            trail.clone(),
            resource_extras(role, false),
        ) else {
            continue;
        };
        if state.admit(relation, limit, candidate) {
            other_sides.push((edge.target, edge.kind, trail));
        }
    }

    // Distance 1: entities injected into the container class (constructor parameter types).
    if let Some(container) = graph.node(seed).and_then(|view| view.attrs.parent) {
        let type_kinds = EdgeKindSet::from_kinds([
            EdgeKind::UsesType,
            EdgeKind::AcceptsType,
            EdgeKind::References,
        ]);
        let mut holders = vec![container];
        holders.extend(
            collect_edges(
                graph,
                container,
                Direction::Out,
                EdgeKindSet::of(EdgeKind::Contains),
            )
            .into_iter()
            .filter(|edge| {
                graph
                    .node(edge.target)
                    .is_some_and(|view| view.kind == NodeKind::Constructor)
            })
            .map(|edge| edge.target),
        );
        let mut entities: Vec<EdgeView> = holders
            .iter()
            .flat_map(|holder| collect_edges(graph, *holder, Direction::Out, type_kinds))
            .filter(|edge| {
                graph
                    .node(edge.target)
                    .is_some_and(|view| view.kind == NodeKind::DatabaseEntity)
            })
            .collect();
        entities.sort_by_key(|edge| (Reverse(edge.confidence), edge.target));
        entities.dedup_by_key(|edge| edge.target);
        let via_container = root.extend(
            step(
                container,
                EdgeKind::Contains,
                seed,
                codegraph::Confidence::MAX,
                side,
            ),
            container,
            false,
        );
        for edge in entities {
            if edge.target == seed {
                continue;
            }
            let trail = via_container.extend(
                step(edge.source, edge.kind, edge.target, edge.confidence, side),
                edge.target,
                true,
            );
            if let Some(candidate) = cx.candidate(
                side,
                edge.target,
                trail,
                resource_extras(ResourceRole::Injected, false),
            ) {
                state.admit(Relation::DbEntity, limit, candidate);
            }
        }
    }

    // Distance 2: the other side of each resource.
    for (resource, kind, trail) in other_sides {
        if trail.min_confidence < cx.budget.min_confidence {
            continue;
        }
        let mut emitted = 0u32;
        match kind {
            EdgeKind::ProducesJob | EdgeKind::Publishes => other_side(
                cx,
                graph,
                state,
                resource,
                &trail,
                EdgeKindSet::of(EdgeKind::ConsumesJob).union(EdgeKindSet::of(EdgeKind::Subscribes)),
                Relation::QueueConsumer,
                ResourceRole::Consumer,
                &mut emitted,
            ),
            EdgeKind::WritesTable => {
                other_side(
                    cx,
                    graph,
                    state,
                    resource,
                    &trail,
                    EdgeKindSet::of(EdgeKind::WritesTable),
                    Relation::DbTable,
                    ResourceRole::CoWriter,
                    &mut emitted,
                );
                other_side(
                    cx,
                    graph,
                    state,
                    resource,
                    &trail,
                    EdgeKindSet::of(EdgeKind::ReadsTable),
                    Relation::DbTable,
                    ResourceRole::CoReader,
                    &mut emitted,
                );
            }
            EdgeKind::ReadsConfig
                if graph
                    .node(resource)
                    .is_some_and(|view| view.kind == NodeKind::EnvironmentVariable) =>
            {
                other_side(
                    cx,
                    graph,
                    state,
                    resource,
                    &trail,
                    EdgeKindSet::of(EdgeKind::ReadsConfig),
                    Relation::EnvVar,
                    ResourceRole::CoReader,
                    &mut emitted,
                );
            }
            _ => {}
        }
    }

    // Removed resource edges: on base, not on head.
    if side == GraphSide::Head {
        if let Some(base) = cx.base {
            let base_root = Trail::seed(seed);
            for (edge, target_kind) in resource_edges(base, seed) {
                if own_identity.contains(&(edge.kind, edge.target)) {
                    continue;
                }
                let Some((relation, role)) = classify(edge.kind, target_kind) else {
                    continue;
                };
                let trail = base_root.extend(
                    step(
                        seed,
                        edge.kind,
                        edge.target,
                        edge.confidence,
                        GraphSide::Base,
                    ),
                    edge.target,
                    true,
                );
                if let Some(candidate) = cx.candidate(
                    GraphSide::Base,
                    edge.target,
                    trail,
                    resource_extras(role, true),
                ) {
                    state.admit(relation, limit, candidate);
                }
            }
        }
    }
}
