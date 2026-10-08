//! The `OVERRIDES` post-pass (CG-005).
//!
//! `OVERRIDES` cannot come out of reference resolution, because resolving `this.render()` in a
//! subclass stops at the subclass's own member table. It is derived instead: for every
//! `EXTENDS`/`IMPLEMENTS` pair, a member of the subtype that has the same name and the same
//! *kind class* as a member of the supertype overrides it.
//!
//! Confidence is `derived(Structural, base_edge)`, i.e. the weaker of "structural" and the
//! inheritance edge it was read off, so an `IMPLEMENTS` edge resolved only by an ambiguous name
//! (0.3) does not produce a confident override.

use std::collections::BTreeMap;

use review_core::location::RepoPath;

use crate::confidence;
use crate::edge::{Edge, EdgeIdentity, Location, Provenance, ResolvedBy};
use crate::edge_kind::EdgeKind;
use crate::graph::NodeInput;
use crate::linker::symbol_table::SymbolTable;
use crate::node_id::NodeKey;
use crate::node_kind::NodeKind;

/// The inheritance edges the pass reads.
const INHERITANCE: [EdgeKind; 2] = [EdgeKind::Extends, EdgeKind::Implements];

/// Produces one `OVERRIDES` edge per matching member pair.
///
/// The member table spans the whole snapshot, because a supertype's members usually live in
/// another file; only the inheritance *edges* are filtered by `file`, so re-linking one file
/// re-derives exactly that file's overrides and touches nothing else (INC-005).
#[must_use]
pub fn overrides_for_file(edges: &[Edge], table: &SymbolTable, file: &RepoPath) -> Vec<Edge> {
    let mut members: BTreeMap<NodeKey, BTreeMap<String, NodeKey>> = BTreeMap::new();
    for (_, info) in table.symbols() {
        let Some(parent) = info.parent else {
            continue;
        };
        members
            .entry(parent)
            .or_default()
            .insert(info.name.clone(), info.key);
    }

    let mut out = Vec::new();
    for edge in edges {
        if !INHERITANCE.contains(&edge.kind) || edge.origin_file.as_ref() != Some(file) {
            continue;
        }
        let (Some(sub_members), Some(super_members)) =
            (members.get(&edge.source), members.get(&edge.target))
        else {
            continue;
        };
        for (name, sub_member) in sub_members {
            let Some(super_member) = super_members.get(name) else {
                continue;
            };
            if !same_kind_class(table.kind_of(sub_member), table.kind_of(super_member)) {
                continue;
            }
            // The override is observed at the subclass member's declaration, not at the
            // inheritance edge that proved it, so a consumer can point a reviewer at the code.
            let line = table
                .symbols()
                .find(|(_, info)| &info.key == sub_member)
                .map_or(1, |(_, info)| info.range.start.line);
            let mut override_edge = Edge::new(
                EdgeKind::Overrides,
                *sub_member,
                *super_member,
                confidence::derived(ResolvedBy::Structural, edge.confidence),
                ResolvedBy::Structural,
                Provenance::Linker,
            );
            if let Some(location) = &edge.location {
                override_edge = override_edge.with_location(Location::new(
                    location.file.clone(),
                    line,
                    location.col,
                ));
            }
            out.push(override_edge.with_origin_file(file.clone()));
        }
    }
    out.sort();
    out
}

/// The pass over a whole snapshot: one call per file's edges, merged and deduplicated.
#[must_use]
pub fn overrides_for_snapshot(
    edges: &[Edge],
    table: &SymbolTable,
    files: &[RepoPath],
) -> Vec<Edge> {
    let mut out: BTreeMap<EdgeIdentity, Edge> = BTreeMap::new();
    for file in files {
        for edge in overrides_for_file(edges, table, file) {
            let identity = edge.identity();
            match out.get_mut(&identity) {
                Some(existing) => existing.merge_occurrence(&edge),
                None => {
                    out.insert(identity, edge);
                }
            }
        }
    }
    out.into_values().collect()
}

/// The kind classes the pass considers interchangeable: an override replaces behaviour, so a
/// method may override a method but not a property, and an accessor pair counts as a method.
#[must_use]
pub fn same_kind_class(sub: Option<NodeKind>, sup: Option<NodeKind>) -> bool {
    let Some(sub) = sub else { return false };
    let Some(sup) = sup else { return false };
    if sub == sup {
        return true;
    }
    match (class_of(sub), class_of(sup)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn class_of(kind: NodeKind) -> Option<u8> {
    match kind {
        NodeKind::Function
        | NodeKind::Method
        | NodeKind::Constructor
        | NodeKind::Handler
        | NodeKind::JobHandler => Some(0),
        NodeKind::Property | NodeKind::Field => Some(1),
        _ => None,
    }
}

/// Applies kind refinements produced by the framework mapper to the linker-produced nodes.
///
/// A refinement only changes the displayed kind; the key is the canonical symbol id's hash, so
/// a refined node keeps its identity (clarification C3).
pub fn apply_refinements(nodes: &mut [NodeInput], refinements: &[(NodeKey, NodeKind)]) {
    if refinements.is_empty() {
        return;
    }
    let map: BTreeMap<NodeKey, NodeKind> = refinements.iter().copied().collect();
    for node in nodes {
        if let Some(kind) = map.get(&node.id.key()) {
            if kind.refines_from(node.kind) {
                node.kind = *kind;
            }
        }
    }
}
