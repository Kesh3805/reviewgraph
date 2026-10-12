//! `db_writes_use_transaction_wrapper` (PROF-004).
//!
//! The population is the service methods with a `DbWriteLike` fact. A method conforms when every
//! write lies inside one of its `TransactionWrapper` facts, or when it has callers and every
//! caller wraps the call (the "caller-owned transaction" case, checked through `CALLS` in-edges at
//! depth 1).

use analysis_ir::facts::{FactKind, SyntaxFact};
use codegraph::{Direction, EdgeKind, NodeKind};

use super::{edges, node_sample};
use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::facts::{contains, fact_line};
use crate::infer::roles::Role;

pub const ID: &str = "db_writes_use_transaction_wrapper";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default)]
pub struct TransactionWrapperMiner;

fn wraps_line(wrappers: &[&SyntaxFact], line: Option<u32>) -> bool {
    match line {
        Some(line) => wrappers
            .iter()
            .any(|w| w.range.start.line <= line && line <= w.range.end.line),
        None => !wrappers.is_empty(),
    }
}

impl ConventionMiner for TransactionWrapperMiner {
    fn id(&self) -> &str {
        ID
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        "Service methods that write to the database run inside a transaction".to_owned()
    }

    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample> {
        let facts = cx.inputs.facts;
        let mut out = Vec::new();
        cx.inputs.graph.for_each_node(&mut |node| {
            if !matches!(node.kind, NodeKind::Method | NodeKind::Function) {
                return;
            }
            if cx.roles.node_role(&node) != Some(Role::Service) {
                return;
            }
            if facts.facts_of_kind(node.key, FactKind::DbWriteLike).next().is_some() {
                out.extend(node_sample(&node, ""));
            }
        });
        out
    }

    fn conforms(&self, sample: &Sample, cx: &MiningContext<'_>) -> Conformance {
        let Some(key) = sample.key else {
            return Conformance::NotApplicable;
        };
        let facts = cx.inputs.facts;
        let wrappers: Vec<&SyntaxFact> =
            facts.facts_of_kind(key, FactKind::TransactionWrapper).collect();
        let unwrapped: Vec<&SyntaxFact> = facts
            .facts_of_kind(key, FactKind::DbWriteLike)
            .filter(|write| !wrappers.iter().any(|w| contains(w, write)))
            .collect();
        let Some(first) = unwrapped.first() else {
            return Conformance::Conforms;
        };
        let graph = cx.inputs.graph;
        let callers = edges(graph, key, Direction::In, &[EdgeKind::Calls]);
        let caller_owned = !callers.is_empty()
            && callers.iter().all(|call| {
                let caller_wrappers: Vec<&SyntaxFact> = facts
                    .facts_of_kind(call.source, FactKind::TransactionWrapper)
                    .collect();
                wraps_line(&caller_wrappers, call.location.map(|(_, line, _)| line))
            });
        if caller_owned {
            return Conformance::Conforms;
        }
        Conformance::Violates {
            evidence: format!(
                "database write at line {} runs outside a transaction",
                fact_line(first)
            ),
        }
    }
}
