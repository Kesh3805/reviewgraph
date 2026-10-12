//! `layer_dependency:<from>-><to>` conventions: the observed layer matrix as conventions
//! (PROF-002). The population is the files of the `from` layer; a file violates when anything in
//! it imports, calls or depends on something in the `to` layer. `samples` is the file count and
//! `violations` the files with such an edge.

use codegraph::{EdgeKind, EdgeKindSet, GraphQueryExt, NodeFlags, NodeKind};

use super::collect;
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
use crate::infer::roles::{classify, Role};

/// `(convention id, from, to)`: dependency directions that are conventionally forbidden.
pub const PAIRS: [(&str, Role, Role); 4] = [
    (
        "layer_dependency:service->controller",
        Role::Service,
        Role::Controller,
    ),
    (
        "layer_dependency:repository->service",
        Role::Repository,
        Role::Service,
    ),
    (
        "layer_dependency:repository->controller",
        Role::Repository,
        Role::Controller,
    ),
    (
        "layer_dependency:entity->service",
        Role::Entity,
        Role::Service,
    ),
];

#[derive(Debug, Clone, Copy)]
pub struct LayerDependency {
    id: &'static str,
    from: Role,
    to: Role,
}

impl LayerDependency {
    pub const fn new(id: &'static str, from: Role, to: Role) -> Self {
        Self { id, from, to }
    }
}

impl ConventionMiner for LayerDependency {
    fn id(&self) -> &'static str {
        self.id
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "A layer does not depend on the layers above it."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        let mut samples: Vec<Sample> = collect(g.graph(), |n| {
            n.kind == NodeKind::File
                && classify(n.kind, NodeFlags::EMPTY, n.file).map(|a| a.role) == Some(self.from)
        })
        .into_iter()
        .map(|f| Sample::new(f.id, f.file.unwrap_or_default()))
        .collect();
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        samples
    }

    fn conforms(&self, s: &Sample, g: &dyn GraphView) -> Conformance {
        let q = g.graph();
        let kinds =
            EdgeKindSet::from_kinds([EdgeKind::Imports, EdgeKind::Calls, EdgeKind::DependsOn]);
        let mut keys = Vec::new();
        q.nodes_in_file(&s.path, &mut |n| keys.push(n.key));
        for key in keys {
            for edge in q.out_edges(key, kinds) {
                let (Some(from), Some(to)) = (q.node(key), q.node(edge.target)) else {
                    continue;
                };
                if classify(to.kind, to.attrs.flags, to.file).map(|a| a.role) == Some(self.to) {
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
    use crate::testgraph::{partial, OwnedView, TestGraph};

    #[test]
    fn repository_depending_on_service_violates() {
        let mut t = TestGraph::new();
        t.node(
            NodeKind::File,
            "f:r1",
            "a.repository.ts",
            Some("src/a/a.repository.ts"),
        )
        .node(
            NodeKind::File,
            "f:r2",
            "b.repository.ts",
            Some("src/b/b.repository.ts"),
        )
        .node(
            NodeKind::File,
            "f:s",
            "a.service.ts",
            Some("src/a/a.service.ts"),
        )
        .edge(EdgeKind::Imports, "f:r1", "f:s");
        let v = OwnedView::new(t.build());
        let miner = LayerDependency::new(PAIRS[1].0, PAIRS[1].1, PAIRS[1].2);
        let samples = miner.population(&v, &partial());
        assert_eq!(samples.len(), 2);
        let bad = samples
            .iter()
            .filter(|s| matches!(miner.conforms(s, &v), Conformance::Violates { .. }))
            .count();
        assert_eq!(bad, 1);
    }
}
