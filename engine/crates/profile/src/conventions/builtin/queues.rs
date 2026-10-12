//! Queue profile and `jobs_have_deterministic_id` (PROF-005).
//!
//! A job-enqueueing call is a `call` syntax fact whose detail carries `queue_add = true` and, when
//! the call passes one, `job_id` = the source text of the `jobId` expression. A deterministic id
//! is one built from identifiers and template literals: no `uuid()`, `Date.now()`,
//! `Math.random()` or `nanoid()`. A call without any `jobId` is a violation too.

use analysis_ir::facts::{FactKind, SyntaxFact};
use analysis_ir::symbol::AttrValue;
use codegraph::{EdgeKind, EdgeKindSet, NodeKind};

use super::{collect, in_sources};
use crate::conventions::miner::{Conformance, ConventionMiner, GraphView, PartialProfile, Sample};
use crate::model::QueueProfile;

pub const ID: &str = "jobs_have_deterministic_id";

/// Lower-case fragments that make a job id random or time based.
const NON_DETERMINISTIC: &[&str] = &[
    "uuid",
    "date.now",
    "new date",
    "math.random",
    "nanoid",
    "randombytes",
    "crypto.random",
];

/// Whether a `jobId` expression is deterministic. Empty is not.
pub fn is_deterministic_job_id(expr: &str) -> bool {
    let lower = expr.trim().to_ascii_lowercase();
    !lower.is_empty() && !NON_DETERMINISTIC.iter().any(|bad| lower.contains(bad))
}

/// One job-enqueueing call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobIdSite {
    pub node_id: String,
    pub path: String,
    /// Position among the enqueue calls of the symbol.
    pub index: usize,
    /// The `jobId` expression, when the call passes one.
    pub job_id: Option<String>,
}

impl JobIdSite {
    /// `None` when the id is deterministic, otherwise why it is not.
    pub fn violation(&self) -> Option<String> {
        match &self.job_id {
            None => Some("queue.add without a jobId".to_owned()),
            Some(expr) if !is_deterministic_job_id(expr) => {
                Some(format!("jobId `{expr}` is random or time based"))
            }
            Some(_) => None,
        }
    }
}

fn is_queue_add(f: &SyntaxFact) -> bool {
    f.kind == FactKind::Call && matches!(f.detail.get("queue_add"), Some(AttrValue::Bool(true)))
}

/// Every enqueue call of the repository, in canonical order.
pub fn queue_add_sites(g: &dyn GraphView) -> Vec<JobIdSite> {
    let mut sites = Vec::new();
    for n in collect(g.graph(), |n| g.facts(n.id).iter().any(is_queue_add)) {
        let calls = g.facts(&n.id).iter().filter(|f| is_queue_add(f));
        for (index, f) in calls.enumerate() {
            let job_id = match f.detail.get("job_id") {
                Some(AttrValue::Str(s)) => Some(s.clone()),
                _ => None,
            };
            sites.push(JobIdSite {
                node_id: n.id.clone(),
                path: n.file.clone().unwrap_or_default(),
                index,
                job_id,
            });
        }
    }
    sites.sort_by(|a, b| (&a.node_id, a.index).cmp(&(&b.node_id, b.index)));
    sites
}

/// The queue section of the profile: queue nodes, who produces and who consumes.
pub fn infer_queues(g: &dyn GraphView) -> QueueProfile {
    let q = g.graph();
    let queues = collect(q, |n| n.kind == NodeKind::Queue);
    let mut names: Vec<String> = queues.iter().map(|n| n.id.clone()).collect();
    let mut producers = Vec::new();
    let mut consumers = Vec::new();
    for queue in &queues {
        for kind in [EdgeKind::ProducesJob, EdgeKind::ConsumesJob] {
            let bucket = if kind == EdgeKind::ProducesJob {
                &mut producers
            } else {
                &mut consumers
            };
            for source in in_sources(q, queue.key, EdgeKindSet::of(kind)) {
                if let Some(node) = q.node(source) {
                    bucket.push(node.name.to_owned());
                }
            }
        }
    }
    for list in [&mut names, &mut producers, &mut consumers] {
        list.sort();
        list.dedup();
    }
    let sites = queue_add_sites(g);
    QueueProfile {
        libraries: Vec::new(),
        idempotency_keys: !sites.is_empty() && sites.iter().all(|s| s.violation().is_none()),
        queues: names,
        producers,
        consumers,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct JobsHaveDeterministicId;

impl ConventionMiner for JobsHaveDeterministicId {
    fn id(&self) -> &'static str {
        ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn rule(&self) -> &'static str {
        "Queue jobs are added with a deterministic jobId (no uuid, timestamp or random value)."
    }

    fn population(&self, g: &dyn GraphView, _p: &PartialProfile) -> Vec<Sample> {
        queue_add_sites(g)
            .into_iter()
            .map(|site| {
                let meta = site.violation().unwrap_or_default();
                Sample::new(format!("{}|queue|{}", site.node_id, site.index), site.path)
                    .with_meta(meta)
            })
            .collect()
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

    fn add(job_id: Option<&str>) -> SyntaxFact {
        let mut detail = vec![("queue_add", AttrValue::Bool(true))];
        if let Some(id) = job_id {
            detail.push(("job_id", AttrValue::Str(id.to_owned())));
        }
        fact(FactKind::Call, "call:this.queue.add/3", &detail)
    }

    fn view() -> OwnedView {
        let mut t = TestGraph::new();
        t.node(NodeKind::Queue, "queue:emails", "emails", None)
            .node(NodeKind::Queue, "queue:reports", "reports", None)
            .node(
                NodeKind::Method,
                "m:send",
                "send",
                Some("src/mail/mail.service.ts"),
            )
            .node(
                NodeKind::Method,
                "m:report",
                "report",
                Some("src/reports/reports.service.ts"),
            )
            .node(
                NodeKind::Class,
                "c:consumer",
                "MailProcessor",
                Some("src/mail/mail.processor.ts"),
            )
            .edge(EdgeKind::ProducesJob, "m:send", "queue:emails")
            .edge(EdgeKind::ProducesJob, "m:report", "queue:reports")
            .edge(EdgeKind::ConsumesJob, "c:consumer", "queue:emails");
        OwnedView::new(t.build())
            .with_facts("m:send", vec![add(Some("`email:${user.id}:${template}`"))])
            .with_facts("m:report", vec![add(Some("uuid()")), add(None)])
    }

    #[test]
    fn deterministic_job_id_detected() {
        assert!(is_deterministic_job_id("`email:${user.id}`"));
        assert!(!is_deterministic_job_id(""));
    }

    #[test]
    fn random_job_id_violates() {
        let v = view();
        let samples = JobsHaveDeterministicId.population(&v, &partial());
        assert_eq!(samples.len(), 3);
        let bad = samples
            .iter()
            .filter(|s| {
                matches!(
                    JobsHaveDeterministicId.conforms(s, &v),
                    Conformance::Violates { .. }
                )
            })
            .count();
        assert_eq!(bad, 2, "the uuid() id and the missing id");
    }

    #[test]
    fn queue_profile_lists_both_queues() {
        let p = infer_queues(&view());
        assert_eq!(
            p.queues,
            vec!["queue:emails".to_owned(), "queue:reports".to_owned()]
        );
        assert_eq!(p.producers, vec!["report".to_owned(), "send".to_owned()]);
        assert_eq!(p.consumers, vec!["MailProcessor".to_owned()]);
        assert!(!p.idempotency_keys);
    }
}
