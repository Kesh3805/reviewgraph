//! `errors_extend_domain_hierarchy` (PROF-004).
//!
//! The population is every `throw new X(...)` in service code. The miner discovers the domain
//! base class as the most frequent in-repository ancestor of the thrown classes (never a built-in
//! such as `Error` or `HttpException`); a throw conforms when `X` is the base or extends it.
//! Without a discoverable base no convention is emitted.

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::facts::FactKind;
use codegraph::{Direction, EdgeKind, GraphQuery, NodeKey, NodeKind};

use super::edges;
use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::facts::{detail_str, fact_line};
use crate::infer::roles::Role;
use crate::model::RepositoryProfile;

pub const ID: &str = "errors_extend_domain_hierarchy";
pub const VERSION: u32 = 1;

/// Ancestors that never count as a domain base.
const BUILTIN_BASES: &[&str] = &["Error", "HttpException", "Exception", "Object"];
const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, Default)]
pub struct ErrorHierarchyMiner;

struct Throw {
    sample: Sample,
    class: String,
}

/// Class name → class keys, for in-repository classes.
fn class_index(graph: &dyn GraphQuery) -> BTreeMap<String, Vec<NodeKey>> {
    let mut out: BTreeMap<String, Vec<NodeKey>> = BTreeMap::new();
    graph.for_each_node(&mut |node| {
        if node.kind == NodeKind::Class && node.file.is_some() {
            out.entry(node.name.to_owned()).or_default().push(node.key);
        }
    });
    out
}

/// In-repository ancestor names of a class (nearest first).
fn ancestors(graph: &dyn GraphQuery, index: &BTreeMap<String, Vec<NodeKey>>, class: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current: Vec<NodeKey> = index.get(class).cloned().unwrap_or_default();
    for _ in 0..MAX_DEPTH {
        let Some(key) = current.first().copied() else {
            break;
        };
        let Some(parent) = edges(graph, key, Direction::Out, &[EdgeKind::Extends])
            .first()
            .and_then(|e| graph.node(e.target))
        else {
            break;
        };
        if !seen.insert(parent.key) {
            break;
        }
        out.push(parent.name.to_owned());
        current = vec![parent.key];
    }
    out
}

fn throws(cx: &MiningContext<'_>) -> Vec<Throw> {
    let facts = cx.inputs.facts;
    let mut out = Vec::new();
    cx.inputs.graph.for_each_node(&mut |node| {
        if !matches!(node.kind, NodeKind::Method | NodeKind::Function | NodeKind::Constructor) {
            return;
        }
        if cx.roles.node_role(&node) != Some(Role::Service) {
            return;
        }
        let Some(path) = node.file else {
            return;
        };
        for fact in facts.facts_of_kind(node.key, FactKind::Throw) {
            if let Some(class) = detail_str(fact, "class") {
                out.push(Throw {
                    sample: Sample {
                        path: path.to_owned(),
                        line: fact_line(fact),
                        symbol: Some(node.id.to_owned()),
                        key: Some(node.key),
                        detail: class.to_owned(),
                    },
                    class: class.to_owned(),
                });
            }
        }
    });
    out
}

/// The discovered base class, if any.
pub fn discover_base(cx: &MiningContext<'_>) -> Option<String> {
    let graph = cx.inputs.graph;
    let index = class_index(graph);
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for throw in throws(cx) {
        for ancestor in ancestors(graph, &index, &throw.class) {
            if !BUILTIN_BASES.contains(&ancestor.as_str()) && index.contains_key(&ancestor) {
                *counts.entry(ancestor).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(name, _)| name)
}

impl ConventionMiner for ErrorHierarchyMiner {
    fn id(&self) -> &str {
        ID
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        "Thrown errors extend the repository's domain error base class".to_owned()
    }

    /// The ancestry walk happens once here; each sample's detail is `class|base|y` or
    /// `class|base|n`.
    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample> {
        let Some(base) = discover_base(cx) else {
            return Vec::new();
        };
        let graph = cx.inputs.graph;
        let index = class_index(graph);
        let mut verdicts: BTreeMap<String, bool> = BTreeMap::new();
        throws(cx)
            .into_iter()
            .map(|t| {
                let ok = *verdicts.entry(t.class.clone()).or_insert_with(|| {
                    t.class == base || ancestors(graph, &index, &t.class).contains(&base)
                });
                Sample {
                    detail: format!("{}|{base}|{}", t.class, if ok { "y" } else { "n" }),
                    ..t.sample
                }
            })
            .collect()
    }

    fn conforms(&self, sample: &Sample, _cx: &MiningContext<'_>) -> Conformance {
        let parts: Vec<&str> = sample.detail.split('|').collect();
        match parts.as_slice() {
            [_, _, "y"] => Conformance::Conforms,
            [class, base, "n"] => Conformance::Violates {
                evidence: format!("throws `{class}`, which does not extend `{base}`"),
            },
            _ => Conformance::NotApplicable,
        }
    }

    fn contribute(&self, cx: &MiningContext<'_>, profile: &mut RepositoryProfile) {
        let classes: BTreeSet<String> = throws(cx).into_iter().map(|t| t.class).collect();
        profile.errors.thrown_classes = u32::try_from(classes.len()).unwrap_or(u32::MAX);
        profile.errors.base_error_class = discover_base(cx);
    }
}
