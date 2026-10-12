//! `@Processor` classes, `@Process` / `process(job)` handlers, queue events and `new Worker`.

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, FrameworkFactKind, IrDecorator, IrExpr, IrSymbol};
use review_core::symbol::SymbolKind;
use tree_sitter::Node;

use super::{origin, queue_name, ADAPTER, PACKAGES};
use crate::frameworks::util::{arguments_of, find_key, node_string, nodes_of_kinds, s};
use crate::frameworks::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::expr::to_ir_expr;

const EVENT_DECORATORS: [&str; 3] = ["OnWorkerEvent", "OnQueueEvent", "OnGlobalQueueEvent"];

const LEGACY_EVENTS: [(&str, &str); 12] = [
    ("OnQueueActive", "active"),
    ("OnQueueCompleted", "completed"),
    ("OnQueueFailed", "failed"),
    ("OnQueueStalled", "stalled"),
    ("OnQueueProgress", "progress"),
    ("OnQueuePaused", "paused"),
    ("OnQueueResumed", "resumed"),
    ("OnQueueCleaned", "cleaned"),
    ("OnQueueDrained", "drained"),
    ("OnQueueRemoved", "removed"),
    ("OnQueueWaiting", "waiting"),
    ("OnQueueError", "error"),
];

fn decorator_origin(ctx: &FrameworkCtx<'_>, d: &IrDecorator, wanted: &str) -> Option<f32> {
    ctx.name_origin(&d.name, &PACKAGES, wanted)
}

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    let symbols = ctx.symbols;
    for class in symbols.iter().filter(|c| c.kind == SymbolKind::Class) {
        class_consumer(ctx, class);
    }
    worker_instances(ctx);
}

fn class_consumer(ctx: &mut FrameworkCtx<'_>, class: &IrSymbol) {
    let symbols = ctx.symbols;
    let Some((decorator, conf)) = class
        .decorators
        .iter()
        .find_map(|d| decorator_origin(ctx, d, "Processor").map(|c| (d, c)))
    else {
        return;
    };
    let (queue_expr, concurrency) = match decorator.args.first() {
        Some(IrExpr::Object(pairs)) => (
            find_key(pairs, "name").cloned(),
            match find_key(pairs, "concurrency") {
                Some(IrExpr::Num(n)) => n.parse::<i64>().ok(),
                _ => None,
            },
        ),
        other => (other.cloned(), None),
    };
    let (queue, dynamic, name_conf) = queue_name(ctx, queue_expr.as_ref());
    let worker_host = class
        .heritage
        .extends
        .iter()
        .any(|e| e.split(['<', '(']).next().unwrap_or("").trim() == "WorkerHost");
    let style = if worker_host {
        "worker_host"
    } else {
        "legacy_bull"
    };
    let process = symbols.iter().find(|m| {
        m.parent == Some(class.local_id) && m.kind == SymbolKind::Method && m.name == "process"
    });
    let mut attrs = BTreeMap::new();
    attrs.insert(
        "symbol".to_owned(),
        AttrValue::Str(ctx.qname(class.local_id)),
    );
    attrs.insert("queue".to_owned(), AttrValue::Str(queue.clone()));
    attrs.insert("queue_dynamic".to_owned(), AttrValue::Bool(dynamic));
    if let Some(c) = concurrency {
        attrs.insert("concurrency".to_owned(), AttrValue::Int(c));
    }
    attrs.insert("style".to_owned(), s(style));
    if let Some(process) = process {
        attrs.insert(
            "process_symbol".to_owned(),
            AttrValue::Str(ctx.qname(process.local_id)),
        );
    }
    let fact_conf = conf.min(name_conf);
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::QueueConsumer,
        Some(class.local_id),
        attrs,
        decorator.range,
        fact_conf,
    );

    for method in symbols
        .iter()
        .filter(|m| m.parent == Some(class.local_id) && m.kind == SymbolKind::Method)
    {
        for d in &method.decorators {
            if let Some(handler_conf) = decorator_origin(ctx, d, "Process") {
                let job = match d.args.first() {
                    None => "*".to_owned(),
                    Some(IrExpr::Object(pairs)) => find_key(pairs, "name")
                        .and_then(|v| ctx.const_str(v))
                        .unwrap_or_else(|| "*".to_owned()),
                    Some(other) => ctx.const_str(other).unwrap_or_else(|| "*".to_owned()),
                };
                handler(
                    ctx,
                    method,
                    &queue,
                    &job,
                    "process_decorator",
                    None,
                    d.range,
                    handler_conf.min(name_conf),
                );
            }
            let event = EVENT_DECORATORS
                .iter()
                .find(|w| decorator_origin(ctx, d, w).is_some())
                .map(|_| {
                    d.args
                        .first()
                        .and_then(|a| ctx.const_str(a))
                        .unwrap_or_else(|| "*".to_owned())
                })
                .or_else(|| {
                    LEGACY_EVENTS
                        .iter()
                        .find(|(w, _)| decorator_origin(ctx, d, w).is_some())
                        .map(|(_, e)| (*e).to_owned())
                });
            if let Some(event) = event {
                handler(
                    ctx,
                    method,
                    &queue,
                    "",
                    "worker_event",
                    Some(event),
                    d.range,
                    fact_conf,
                );
            }
        }
    }

    if worker_host {
        if let Some(process) = process {
            process_switch(ctx, process, &queue, fact_conf);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handler(
    ctx: &mut FrameworkCtx<'_>,
    method: &IrSymbol,
    queue: &str,
    job: &str,
    via: &str,
    event: Option<String>,
    range: review_core::location::SourceRange,
    confidence: f32,
) {
    let mut attrs = BTreeMap::new();
    attrs.insert(
        "symbol".to_owned(),
        AttrValue::Str(ctx.qname(method.local_id)),
    );
    attrs.insert("queue".to_owned(), s(queue));
    if event.is_none() {
        attrs.insert("job".to_owned(), s(job));
    }
    attrs.insert("via".to_owned(), s(via));
    if let Some(event) = event {
        attrs.insert("event".to_owned(), AttrValue::Str(event));
    }
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::QueueJobHandler,
        Some(method.local_id),
        attrs,
        range,
        confidence,
    );
}

/// Each `case 'x':` of `switch (job.name)` inside `process(job)` is a handler for job `x`; no
/// switch means one `*` handler.
fn process_switch(ctx: &mut FrameworkCtx<'_>, process: &IrSymbol, queue: &str, conf: f32) {
    let mut cases: Vec<(String, review_core::location::SourceRange)> = Vec::new();
    for switch in nodes_of_kinds(ctx.root, &[kind::SWITCH_STATEMENT]) {
        let range = ctx.src.range(switch);
        if ctx.owner(range.start) != process.local_id {
            continue;
        }
        let discriminant = switch
            .child_by_field_name(field::VALUE)
            .map(|v| ctx.src.text(v).into_owned())
            .unwrap_or_default();
        let discriminant = discriminant
            .trim_matches(|c| c == '(' || c == ')')
            .trim()
            .to_owned();
        if !discriminant.ends_with(".name") {
            continue;
        }
        for case in nodes_of_kinds(switch, &[kind::SWITCH_CASE]) {
            if !is_direct_case(switch, case) {
                continue;
            }
            let value = case
                .child_by_field_name(field::VALUE)
                .and_then(|v| node_string(v, ctx.src));
            if let Some(job) = value {
                cases.push((job, ctx.src.range(case)));
            }
        }
    }
    if cases.is_empty() {
        handler(
            ctx,
            process,
            queue,
            "*",
            "worker_host_process",
            None,
            process.range,
            conf.min(0.8),
        );
        return;
    }
    for (job, range) in cases {
        handler(
            ctx,
            process,
            queue,
            &job,
            "switch_case",
            None,
            range,
            conf.min(0.8),
        );
    }
}

fn is_direct_case(switch: Node<'_>, case: Node<'_>) -> bool {
    case.parent()
        .and_then(|body| body.parent())
        .is_some_and(|p| p.id() == switch.id())
}

/// `new Worker('queue', handler)` is a consumer without a class.
fn worker_instances(ctx: &mut FrameworkCtx<'_>) {
    for new_expr in nodes_of_kinds(ctx.root, &[kind::NEW_EXPRESSION]) {
        let Some(constructor) = new_expr.child_by_field_name(field::CONSTRUCTOR) else {
            continue;
        };
        let name = ctx.src.text(constructor).into_owned();
        if name.rsplit('.').next() != Some("Worker") {
            continue;
        }
        let Some(conf) = origin(ctx, &name, "Worker") else {
            continue;
        };
        let first = arguments_of(new_expr)
            .into_iter()
            .next()
            .map(|a| to_ir_expr(a, ctx.src));
        let (queue, dynamic, name_conf) = queue_name(ctx, first.as_ref());
        let range = ctx.src.range(new_expr);
        let owner = ctx.owner(range.start);
        let mut attrs = BTreeMap::new();
        attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
        attrs.insert("queue".to_owned(), AttrValue::Str(queue));
        attrs.insert("queue_dynamic".to_owned(), AttrValue::Bool(dynamic));
        attrs.insert("style".to_owned(), s("worker_instance"));
        ctx.emit(
            ADAPTER,
            FrameworkFactKind::QueueConsumer,
            Some(owner),
            attrs,
            range,
            conf.min(name_conf),
        );
    }
}
