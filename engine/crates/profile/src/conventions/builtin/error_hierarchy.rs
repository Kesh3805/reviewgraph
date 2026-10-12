//! `errors_extend_domain_hierarchy` (PROF-004).
//!
//! The population is `throw new X(...)` sites in services. The miner discovers the base class as
//! the most frequent ancestor (or class) among the thrown classes, ignoring built-ins such as
//! `Error` and `HttpException`. A throw conforms when `X` is, or extends, that base.

use std::collections::{BTreeMap, BTreeSet};

use analysis_ir::facts::FactKind;
use codegraph::query::GraphQuery;
use codegraph::{EdgeKind, EdgeKindSet, NodeKey, NodeKind};

use super::{collect, out_targets};
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
use crate::infer::roles::{classify, Role};

pub const ID: &str = "errors_extend_domain_hierarchy";

/// Built-in error types that never count as a domain base.
const BUILTIN_ERRORS: &[&str] = &[
    "Error",
    "Exception",
    "HttpException",
    "RpcException",
    "WsException",
    "TypeError",
    "RangeError",
    "SyntaxError",
];

/// A base needs at least this many throw sites to be called a convention.
const MIN_BASE_SITES: usize = 2;
/// Inheritance chains are followed this far.
const MAX_DEPTH: usize = 8;

#[derive(Debug, Default, Clone, Copy)]
pub struct ErrorHierarchy;

#[derive(Debug)]
struct Site {
    node_id: String,
    path: String,
    index: usize,
    class: String,
    /// The class and its non-built-in ancestors; `None` when the class is not in the graph.
    lineage: Option<Vec<String>>,
}

/// The discovered base and every analyzed throw site.
#[derive(Debug)]
pub struct Analysis {
    pub base: Option<String>,
    sites: Vec<Site>,
}

fn is_builtin(name: &str) -> bool {
    BUILTIN_ERRORS.contains(&name)
}

/// `name`, then its ancestors along `EXTENDS`, nearest first, without built-ins.
fn lineage(q: &dyn GraphQuery, start: NodeKey, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !is_builtin(name) {
        out.push(name.to_owned());
    }
    let mut frontier = vec![start];
    let mut seen: BTreeSet<NodeKey> = BTreeSet::from([start]);
    for _ in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for key in frontier {
            for parent in out_targets(q, key, EdgeKindSet::of(EdgeKind::Extends)) {
                if !seen.insert(parent) {
                    continue;
                }
                if let Some(p) = q.node(parent) {
                    if !is_builtin(p.name) {
                        out.push(p.name.to_owned());
                    }
                }
                next.push(parent);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

/// Finds the throw sites of services and the base class they share.
pub fn analyze(g: &dyn GraphView) -> Analysis {
    let q = g.graph();
    let mut class_nodes: BTreeMap<String, NodeKey> = BTreeMap::new();
    for c in collect(q, |n| matches!(n.kind, NodeKind::Class) && n.file.is_some()) {
        class_nodes.entry(c.name).or_insert(c.key);
    }

    let throwers = collect(q, |n| {
        matches!(
            n.kind,
            NodeKind::Method | NodeKind::Function | NodeKind::Constructor | NodeKind::Handler
        ) && classify(n.kind, n.attrs.flags, n.file).map(|a| a.role) == Some(Role::Service)
            && g.facts(n.id).iter().any(|f| f.kind == FactKind::Throw)
    });
    let mut sites = Vec::new();
    for t in throwers {
        let throws = g.facts(&t.id).iter().filter(|f| f.kind == FactKind::Throw);
        for (index, fact) in throws.enumerate() {
            let Some(class) = fact.key.strip_prefix("throw:") else {
                continue;
            };
            if !class.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                continue;
            }
            let lineage = class_nodes.get(class).map(|key| lineage(q, *key, class));
            sites.push(Site {
                node_id: t.id.clone(),
                path: t.file.clone().unwrap_or_default(),
                index,
                class: class.to_owned(),
                lineage,
            });
        }
    }

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for site in &sites {
        if let Some(lineage) = &site.lineage {
            for name in lineage.iter().collect::<BTreeSet<_>>() {
                *counts.entry(name.as_str()).or_insert(0) += 1;
            }
        }
    }
    let base = counts
        .iter()
        .filter(|(_, n)| **n >= MIN_BASE_SITES)
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(name, _)| (*name).to_owned());
    Analysis { base, sites }
}

impl ConventionMiner for ErrorHierarchy {
    fn id(&self) -> &'static str {
        ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "Errors thrown by services extend the repository's domain error hierarchy."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        let analysis = analyze(g);
        let Some(base) = analysis.base else {
            return Vec::new();
        };
        let mut samples: Vec<Sample> = analysis
            .sites
            .into_iter()
            .map(|site| {
                let conforms = site
                    .lineage
                    .as_ref()
                    .is_some_and(|l| l.iter().any(|n| *n == base));
                let meta = if conforms {
                    String::new()
                } else {
                    format!("throws {}, which does not extend {base}", site.class)
                };
                Sample::new(format!("{}|throw|{}", site.node_id, site.index), site.path)
                    .with_meta(meta)
            })
            .collect();
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        samples
    }

    fn conforms(&self, s: &Sample, _g: &dyn GraphView) -> Conformance {
        if s.meta.is_empty() {
            Conformance::Conforms
        } else {
            Conformance::Violates {
                evidence: s.meta.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgraph::{fact, partial, OwnedView, TestGraph};

    fn view() -> OwnedView {
        let mut t = TestGraph::new();
        t.node(
            NodeKind::Class,
            "c:domain",
            "DomainError",
            Some("src/common/errors.ts"),
        )
        .node(
            NodeKind::Class,
            "c:notfound",
            "OrderNotFound",
            Some("src/orders/errors.ts"),
        )
        .node(
            NodeKind::Class,
            "c:invalid",
            "InvalidOrder",
            Some("src/orders/errors.ts"),
        )
        .node(
            NodeKind::Class,
            "c:stray",
            "StrayError",
            Some("src/misc/errors.ts"),
        )
        .node(
            NodeKind::Class,
            "c:http",
            "HttpException",
            Some("node_modules/@nestjs/common.ts"),
        )
        .edge(EdgeKind::Extends, "c:notfound", "c:domain")
        .edge(EdgeKind::Extends, "c:invalid", "c:domain")
        .edge(EdgeKind::Extends, "c:stray", "c:http")
        .node(
            NodeKind::Method,
            "m:a",
            "create",
            Some("src/orders/orders.service.ts"),
        )
        .node(
            NodeKind::Method,
            "m:b",
            "cancel",
            Some("src/orders/orders.service.ts"),
        )
        .node(
            NodeKind::Method,
            "m:c",
            "stray",
            Some("src/misc/misc.service.ts"),
        );
        OwnedView::new(t.build())
            .with_facts(
                "m:a",
                vec![fact(FactKind::Throw, "throw:OrderNotFound", &[])],
            )
            .with_facts(
                "m:b",
                vec![fact(FactKind::Throw, "throw:InvalidOrder", &[])],
            )
            .with_facts(
                "m:c",
                vec![
                    fact(FactKind::Throw, "throw:StrayError", &[]),
                    fact(FactKind::Throw, "throw:NotFoundException", &[]),
                ],
            )
    }

    #[test]
    fn error_base_discovered() {
        let v = view();
        assert_eq!(analyze(&v).base.as_deref(), Some("DomainError"));
    }

    #[test]
    fn non_hierarchy_throws_violate() {
        let v = view();
        let samples = ErrorHierarchy.population(&v, &partial());
        assert_eq!(samples.len(), 4);
        let bad: Vec<_> = samples
            .iter()
            .filter(|s| matches!(ErrorHierarchy.conforms(s, &v), Conformance::Violates { .. }))
            .collect();
        assert_eq!(
            bad.len(),
            2,
            "StrayError and the external NotFoundException"
        );
    }

    #[test]
    fn no_shared_base_means_no_population() {
        let mut t = TestGraph::new();
        t.node(NodeKind::Class, "c:one", "OneError", Some("src/a/e.ts"))
            .node(NodeKind::Method, "m:a", "go", Some("src/a/a.service.ts"));
        let v = OwnedView::new(t.build())
            .with_facts("m:a", vec![fact(FactKind::Throw, "throw:OneError", &[])]);
        assert!(ErrorHierarchy.population(&v, &partial()).is_empty());
    }
}
