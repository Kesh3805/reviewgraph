//! `controllers_do_not_access_repositories` (PROF-004).
//!
//! The population is controller classes. A controller violates when a `DEPENDS_ON` or `CALLS`
//! edge of the class (or of one of its members) reaches a repository-role symbol or a TypeORM
//! `Repository<T>` injection. Controllers recognized only by file name (no framework-refined
//! class) are sampled per file.

use std::collections::BTreeSet;

use codegraph::query::GraphQuery;
use codegraph::{EdgeKind, EdgeKindSet, GraphQueryExt, NodeFlags, NodeKey, NodeKind, NodeRef};

use super::{collect, out_targets};
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
use crate::infer::roles::{classify, Role};

pub const ID: &str = "controllers_do_not_access_repositories";

#[derive(Debug, Default, Clone, Copy)]
pub struct ControllerRepository;

/// A repository-role symbol, or a TypeORM `Repository<T>` handle.
fn is_repository(n: &NodeRef<'_>) -> bool {
    if classify(n.kind, n.attrs.flags, n.file).map(|a| a.role) == Some(Role::Repository) {
        return true;
    }
    n.name == "Repository"
        || n.name.starts_with("Repository<")
        || (matches!(n.kind, NodeKind::Class | NodeKind::Interface)
            && n.name.ends_with("Repository"))
}

/// The nodes whose outgoing edges count for a sample: a controller class and its members, or
/// every node of a controller file.
fn scope_nodes(q: &dyn GraphQuery, key: NodeKey, kind: NodeKind, path: &str) -> Vec<NodeKey> {
    if kind == NodeKind::File {
        let mut keys = Vec::new();
        q.nodes_in_file(path, &mut |n| keys.push(n.key));
        return keys;
    }
    let mut keys = vec![key];
    keys.extend(out_targets(
        q,
        key,
        EdgeKindSet::of(EdgeKind::Contains).union(EdgeKindSet::of(EdgeKind::Declares)),
    ));
    keys
}

impl ConventionMiner for ControllerRepository {
    fn id(&self) -> &'static str {
        ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "Controllers do not depend on repositories; they go through services."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        let q = g.graph();
        let controllers = collect(q, |n| n.kind == NodeKind::Controller);
        let covered: BTreeSet<&str> = controllers
            .iter()
            .filter_map(|c| c.file.as_deref())
            .collect();
        let mut samples: Vec<Sample> = controllers
            .iter()
            .map(|c| Sample::new(c.id.clone(), c.file.clone().unwrap_or_default()))
            .collect();
        for f in collect(q, |n| n.kind == NodeKind::File) {
            let Some(path) = f.file.as_deref() else {
                continue;
            };
            if covered.contains(path) {
                continue;
            }
            let role = classify(NodeKind::File, NodeFlags::EMPTY, Some(path)).map(|a| a.role);
            if role == Some(Role::Controller) {
                samples.push(Sample::new(f.id.clone(), path));
            }
        }
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        samples
    }

    fn conforms(&self, s: &Sample, g: &dyn GraphView) -> Conformance {
        let q = g.graph();
        let Some(node) = q.node_by_id(&s.id) else {
            return Conformance::NotApplicable;
        };
        let deps = EdgeKindSet::of(EdgeKind::DependsOn).union(EdgeKindSet::of(EdgeKind::Calls));
        for key in scope_nodes(q, node.key, node.kind, &s.path) {
            for edge in q.out_edges(key, deps) {
                let (Some(from), Some(to)) = (q.node(key), q.node(edge.target)) else {
                    continue;
                };
                if is_repository(&to) {
                    return Conformance::Violates {
                        evidence: format!("{} -> {} ({})", from.name, to.name, edge.kind),
                    };
                }
            }
        }
        Conformance::Conforms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgraph::{OwnedView, TestGraph};

    fn view(violate: bool) -> OwnedView {
        let mut t = TestGraph::new();
        for m in ["a", "b", "c"] {
            let ctrl = format!("cls:{m}.ctrl");
            let svc = format!("cls:{m}.svc");
            let (cpath, spath) = (
                format!("src/{m}/{m}.controller.ts"),
                format!("src/{m}/{m}.service.ts"),
            );
            t.node(NodeKind::Controller, &ctrl, "Ctrl", Some(cpath.as_str()))
                .node(NodeKind::Class, &svc, "Svc", Some(spath.as_str()))
                .edge(EdgeKind::DependsOn, &ctrl, &svc);
        }
        if violate {
            t.node(
                NodeKind::Class,
                "cls:a.repo",
                "AccountRepository",
                Some("src/a/a.repository.ts"),
            )
            .edge(EdgeKind::DependsOn, "cls:a.ctrl", "cls:a.repo");
        }
        OwnedView::new(t.build())
    }

    #[test]
    fn controller_repository_violation_detected() {
        let v = view(true);
        let m = ControllerRepository;
        let samples = m.population(&v, &crate::testgraph::partial());
        assert_eq!(samples.len(), 3);
        let verdicts: Vec<Conformance> = samples.iter().map(|s| m.conforms(s, &v)).collect();
        let bad: Vec<&Conformance> = verdicts
            .iter()
            .filter(|c| matches!(c, Conformance::Violates { .. }))
            .collect();
        assert_eq!(bad.len(), 1);
        assert!(
            matches!(bad[0], Conformance::Violates { evidence } if evidence.contains("AccountRepository"))
        );
    }

    #[test]
    fn clean_controllers_all_conform() {
        let v = view(false);
        let m = ControllerRepository;
        for s in m.population(&v, &crate::testgraph::partial()) {
            assert_eq!(m.conforms(&s, &v), Conformance::Conforms);
        }
    }

    #[test]
    fn file_named_controllers_are_sampled_too() {
        let mut t = TestGraph::new();
        t.node(
            NodeKind::File,
            "file:x.controller",
            "x.controller.ts",
            Some("src/x/x.controller.ts"),
        );
        let v = OwnedView::new(t.build());
        assert_eq!(
            ControllerRepository
                .population(&v, &crate::testgraph::partial())
                .len(),
            1
        );
    }
}
