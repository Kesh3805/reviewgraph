//! Producers and consumers → `Queue`, `PRODUCES_JOB`, `CONSUMES_JOB` (CG-006).
//!
//! A producer and a consumer of the same queue must land on *one* queue node: that is what makes
//! "who enqueues work nobody reads" a reachable question. The id comes from CG-001's
//! `queue:{name}` scheme, so both sides derive it independently and agree.

use crate::edge::EdgeFlags;
use crate::edge_kind::EdgeKind;
use crate::node_id::NodeId;
use crate::node_kind::NodeKind;

use super::contract::{self, FactCategory, FactIssueCode};
use super::Mapping;

/// `queue_producer` → `Queue` node + `PRODUCES_JOB`.
pub(crate) fn apply_producer(ctx: &mut Mapping<'_>) {
    let Some(producer) = ctx.require_symbol("symbol") else {
        return;
    };
    let Some(queue_key) = queue_node(ctx, FactCategory::QueueProducer) else {
        return;
    };
    ctx.edge(EdgeKind::ProducesJob, producer, queue_key, EdgeFlags::EMPTY);
    if NodeKind::QueueProducer.refines_from(ctx.kind_of(&producer).unwrap_or(NodeKind::Method)) {
        ctx.refine(producer, NodeKind::QueueProducer);
    }
}

/// `queue_consumer` → `Queue` node + `CONSUMES_JOB`; a class refines to `QueueConsumer`, a
/// handler method to `JobHandler`.
pub(crate) fn apply_consumer(ctx: &mut Mapping<'_>) {
    let Some(consumer) = ctx.require_symbol("symbol") else {
        return;
    };
    let Some(queue_key) = queue_node(ctx, FactCategory::QueueConsumer) else {
        return;
    };
    ctx.edge(EdgeKind::ConsumesJob, consumer, queue_key, EdgeFlags::EMPTY);
    let current = ctx.kind_of(&consumer).unwrap_or(NodeKind::Class);
    if NodeKind::JobHandler.refines_from(current) {
        ctx.refine(consumer, NodeKind::JobHandler);
    } else if NodeKind::QueueConsumer.refines_from(current) {
        ctx.refine(consumer, NodeKind::QueueConsumer);
    }
}

/// The shared `queue:{name}` node, plus the job name this fact contributes to its attribute set.
fn queue_node(ctx: &mut Mapping<'_>, category: FactCategory) -> Option<crate::node_id::NodeKey> {
    let mut issues = Vec::new();
    let name = contract::require_str(ctx.fact, category, ctx.path, "queue", &mut issues);
    let job = contract::optional_str(ctx.fact, category, ctx.path, "job", &mut issues);
    for issue in issues {
        ctx.out.issues.push(issue);
    }
    let name = name?;
    let id = match NodeId::queue(&name) {
        Ok(id) => id,
        Err(error) => {
            ctx.issue(
                FactIssueCode::InvalidAttribute,
                format!("queue attribute {name:?} is not usable: {error}"),
            );
            return None;
        }
    };
    if let Some(job) = job {
        ctx.job_names.entry(id.key()).or_default().push(job);
    }
    Some(ctx.node(id, NodeKind::Queue, name.clone(), name))
}
