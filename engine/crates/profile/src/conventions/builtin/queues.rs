//! Queue profile and `jobs_have_deterministic_id` (PROF-005).
//!
//! The population is every job-producing call (`queue.add(name, data, { jobId })`) inside a
//! symbol with a `PRODUCES_JOB` edge. A deterministic job id is a `jobId` expression with no
//! random source (`uuid()`, `Date.now()`, `Math.random()`, `nanoid()`, ...), typically a template
//! literal over identifiers.

use analysis_ir::facts::FactKind;
use codegraph::{Direction, EdgeKind, NodeKind};

use super::{edges, nodes_of_kind};
use crate::conventions::miner::{Conformance, ConventionMiner, MiningContext, Sample};
use crate::facts::{detail_flag, detail_str, fact_line};
use crate::model::{QueueInfo, RepositoryProfile};

pub const ID: &str = "jobs_have_deterministic_id";
pub const VERSION: u32 = 1;

/// Expressions that make a job id random.
const RANDOM_SOURCES: &[&str] = &[
    "uuid",
    "randomuuid",
    "date.now",
    "math.random",
    "nanoid",
    "cuid",
    "ulid",
    "new date",
    "randombytes",
    "performance.now",
    "process.hrtime",
];

/// Whether a `jobId` expression is deterministic.
pub fn is_deterministic_job_id(expr: &str) -> bool {
    let lowered = expr.to_ascii_lowercase();
    let trimmed = lowered.trim();
    if trimmed.is_empty() || trimmed == "undefined" || trimmed == "null" {
        return false;
    }
    !RANDOM_SOURCES.iter().any(|source| lowered.contains(source))
}

/// Whether a call fact enqueues a job.
pub fn is_job_call(fact: &analysis_ir::facts::SyntaxFact) -> bool {
    fact.kind == FactKind::Call
        && (detail_flag(fact, "queue_add")
            || detail_str(fact, "callee")
                .is_some_and(|c| c.ends_with(".add") || c.ends_with(".addBulk")))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct QueuesMiner;

impl ConventionMiner for QueuesMiner {
    fn id(&self) -> &str {
        ID
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn rule(&self) -> String {
        "Enqueued jobs carry a deterministic jobId".to_owned()
    }

    fn population(&self, cx: &MiningContext<'_>) -> Vec<Sample> {
        let graph = cx.inputs.graph;
        let facts = cx.inputs.facts;
        let mut out = Vec::new();
        graph.for_each_node(&mut |node| {
            let Some(path) = node.file else {
                return;
            };
            if edges(graph, node.key, Direction::Out, &[EdgeKind::ProducesJob]).is_empty() {
                return;
            }
            for fact in facts.facts(node.key).iter().filter(|f| is_job_call(f)) {
                out.push(Sample {
                    path: path.to_owned(),
                    line: fact_line(fact),
                    symbol: Some(node.id.to_owned()),
                    key: Some(node.key),
                    detail: detail_str(fact, "job_id").unwrap_or_default().to_owned(),
                });
            }
        });
        out
    }

    fn conforms(&self, sample: &Sample, _cx: &MiningContext<'_>) -> Conformance {
        if sample.detail.is_empty() {
            Conformance::Violates {
                evidence: format!("job enqueued at line {} without a jobId", sample.line),
            }
        } else if is_deterministic_job_id(&sample.detail) {
            Conformance::Conforms
        } else {
            Conformance::Violates {
                evidence: format!("random jobId `{}` at line {}", sample.detail, sample.line),
            }
        }
    }

    fn contribute(&self, cx: &MiningContext<'_>, profile: &mut RepositoryProfile) {
        let graph = cx.inputs.graph;
        let names = |kind: EdgeKind, queue| -> Vec<String> {
            let mut out: Vec<String> = edges(graph, queue, Direction::In, &[kind])
                .iter()
                .filter_map(|e| graph.node(e.source))
                .map(|n| n.qualified_name.to_owned())
                .collect();
            out.sort();
            out.dedup();
            out
        };
        profile.queues.queues = nodes_of_kind(graph, &[NodeKind::Queue])
            .into_iter()
            .map(|queue| QueueInfo {
                id: queue.id.to_owned(),
                name: queue.name.to_owned(),
                producers: names(EdgeKind::ProducesJob, queue.key),
                consumers: names(EdgeKind::ConsumesJob, queue.key),
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_determinism() {
        assert!(is_deterministic_job_id("`invoice:${invoice.id}`"));
        assert!(is_deterministic_job_id("order.id"));
        assert!(!is_deterministic_job_id("uuid()"));
        assert!(!is_deterministic_job_id("`job-${Date.now()}`"));
        assert!(!is_deterministic_job_id("crypto.randomUUID()"));
        assert!(!is_deterministic_job_id(""));
    }
}
