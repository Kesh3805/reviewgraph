//! `db_writes_use_transaction_wrapper` (PROF-004).
//!
//! The population is service methods with a `DbWriteLike` syntax fact. A method conforms when it
//! carries a `TransactionWrapper` fact (`dataSource.transaction`, `@Transactional`, a declared
//! wrapper), or when it is caller-owned: it has callers and every direct caller wraps.

use analysis_ir::facts::FactKind;
use codegraph::{EdgeKind, EdgeKindSet, NodeKind};

use super::{collect, in_sources};
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
use crate::infer::roles::{classify, Role};

pub const ID: &str = "db_writes_use_transaction_wrapper";

#[derive(Debug, Default, Clone, Copy)]
pub struct TransactionWrapper;

fn wraps(g: &dyn GraphView, node_id: &str) -> bool {
    g.facts(node_id)
        .iter()
        .any(|f| f.kind == FactKind::TransactionWrapper)
}

impl ConventionMiner for TransactionWrapper {
    fn id(&self) -> &'static str {
        ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "Service methods that write to the database run inside a transaction wrapper."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        let mut samples: Vec<Sample> = collect(g.graph(), |n| {
            matches!(n.kind, NodeKind::Method | NodeKind::Function)
                && classify(n.kind, n.attrs.flags, n.file).map(|a| a.role) == Some(Role::Service)
                && g.facts(n.id)
                    .iter()
                    .any(|f| f.kind == FactKind::DbWriteLike)
        })
        .into_iter()
        .map(|f| Sample::new(f.id, f.file.unwrap_or_default()))
        .collect();
        samples.sort_by(|a, b| a.id.cmp(&b.id));
        samples
    }

    fn conforms(&self, s: &Sample, g: &dyn GraphView) -> Conformance {
        if wraps(g, &s.id) {
            return Conformance::Conforms;
        }
        let q = g.graph();
        let Some(node) = q.node_by_id(&s.id) else {
            return Conformance::NotApplicable;
        };
        let callers = in_sources(q, node.key, EdgeKindSet::of(EdgeKind::Calls));
        let caller_owned = !callers.is_empty()
            && callers
                .iter()
                .all(|c| q.node(*c).is_some_and(|n| wraps(g, n.id)));
        if caller_owned {
            return Conformance::Conforms;
        }
        let write = g
            .facts(&s.id)
            .iter()
            .find(|f| f.kind == FactKind::DbWriteLike)
            .map_or("", |f| f.key.as_str());
        Conformance::Violates {
            evidence: format!("{write} outside a transaction wrapper"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgraph::{fact, partial, OwnedView, TestGraph};

    fn base() -> TestGraph {
        let mut t = TestGraph::new();
        t.node(
            NodeKind::Method,
            "m:own",
            "ownTx",
            Some("src/a/a.service.ts"),
        )
        .node(
            NodeKind::Method,
            "m:callee",
            "write",
            Some("src/a/a.service.ts"),
        )
        .node(
            NodeKind::Method,
            "m:caller",
            "outer",
            Some("src/a/a.service.ts"),
        )
        .node(
            NodeKind::Method,
            "m:bare",
            "bare",
            Some("src/a/a.service.ts"),
        )
        .edge(EdgeKind::Calls, "m:caller", "m:callee");
        t
    }

    fn view() -> OwnedView {
        let write = || vec![fact(FactKind::DbWriteLike, "db_write:repo.save", &[])];
        let tx = || fact(FactKind::TransactionWrapper, "tx:@Transactional", &[]);
        OwnedView::new(base().build())
            .with_facts("m:own", {
                let mut f = write();
                f.push(tx());
                f
            })
            .with_facts("m:callee", write())
            .with_facts("m:caller", vec![tx()])
            .with_facts("m:bare", write())
    }

    #[test]
    fn transaction_caller_owned_conforms() {
        let v = view();
        let samples = TransactionWrapper.population(&v, &partial());
        assert_eq!(samples.len(), 3);
        let verdict = |id: &str| {
            let s = samples.iter().find(|s| s.id == id).unwrap();
            TransactionWrapper.conforms(s, &v)
        };
        assert_eq!(verdict("m:own"), Conformance::Conforms);
        assert_eq!(
            verdict("m:callee"),
            Conformance::Conforms,
            "every caller wraps"
        );
    }

    #[test]
    fn transaction_missing_violates() {
        let v = view();
        let samples = TransactionWrapper.population(&v, &partial());
        let s = samples.iter().find(|s| s.id == "m:bare").unwrap();
        assert!(matches!(
            TransactionWrapper.conforms(s, &v),
            Conformance::Violates { evidence } if evidence.contains("db_write:repo.save")
        ));
    }

    #[test]
    fn non_service_writes_are_not_sampled() {
        let mut t = TestGraph::new();
        t.node(
            NodeKind::Method,
            "m:repo",
            "save",
            Some("src/a/a.repository.ts"),
        );
        let v = OwnedView::new(t.build()).with_facts(
            "m:repo",
            vec![fact(FactKind::DbWriteLike, "db_write:x", &[])],
        );
        assert!(TransactionWrapper.population(&v, &partial()).is_empty());
    }
}
