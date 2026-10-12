//! `queue.add`, `queue.addBulk`, `queue.upsertJobScheduler` and `FlowProducer.add` producers.

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, FrameworkFactKind, IrExpr};
use review_core::location::SourceRange;

use super::{origin, queue_name, ADAPTER};
use crate::frameworks::nestjs::{injected_properties, InjectedMap, TokenKind};
use crate::frameworks::util::{
    arguments_of, expr_kind, expr_text, find_key, nodes_of_kinds, receiver_types, s,
};
use crate::frameworks::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::db_heuristics::type_head;
use crate::visit::expr::to_ir_expr;
use crate::visit::fact_keys::callee_of;

/// Elements of one `addBulk` call that get their own fact.
const MAX_BULK: usize = 50;
/// Nesting of flow children that is followed.
const MAX_FLOW_DEPTH: usize = 3;

struct Producer {
    queue: String,
    dynamic: bool,
    job: String,
    method: &'static str,
    opts: Vec<(String, IrExpr)>,
    receiver_type: Option<String>,
    confidence: f32,
}

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    let types = receiver_types(ctx.symbols);
    let injected = injected_properties(ctx);
    let queue_vars = queue_vars(ctx);
    for call in nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]) {
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            continue;
        };
        let Some(callee) = callee_of(function, ctx.src) else {
            continue;
        };
        if !matches!(
            callee.name.as_str(),
            "add" | "addBulk" | "upsertJobScheduler"
        ) {
            continue;
        }
        let range = ctx.src.range(call);
        let Some((queue, dynamic, receiver_conf, receiver_type, is_flow)) =
            qualify(ctx, &types, &injected, &queue_vars, &callee.receiver, range)
        else {
            continue;
        };
        let args: Vec<IrExpr> = arguments_of(call)
            .into_iter()
            .map(|a| to_ir_expr(a, ctx.src))
            .collect();
        let mut out: Vec<Producer> = Vec::new();
        match (callee.name.as_str(), is_flow) {
            ("add", true) => {
                if let Some(arg) = args.first() {
                    flow(ctx, arg, 1, &receiver_type, &mut out);
                }
            }
            ("addBulk" | "upsertJobScheduler", true) => {}
            ("add", false) => {
                let (job, job_dynamic) = job_name(ctx, args.first());
                out.push(Producer {
                    queue: queue.clone(),
                    dynamic,
                    job,
                    method: "add",
                    opts: object_of(args.get(2)),
                    receiver_type: receiver_type.clone(),
                    confidence: if job_dynamic {
                        receiver_conf.min(0.5)
                    } else {
                        receiver_conf
                    },
                });
            }
            ("addBulk", _) => {
                let items: Vec<&IrExpr> = match args.first() {
                    Some(IrExpr::Array(items)) => items.iter().take(MAX_BULK).collect(),
                    _ => Vec::new(),
                };
                let named: Vec<&IrExpr> = items
                    .into_iter()
                    .filter(|i| matches!(i, IrExpr::Object(p) if find_key(p, "name").is_some()))
                    .collect();
                if named.is_empty() {
                    out.push(Producer {
                        queue: queue.clone(),
                        dynamic,
                        job: "{bulk}".to_owned(),
                        method: "add_bulk",
                        opts: Vec::new(),
                        receiver_type: receiver_type.clone(),
                        confidence: receiver_conf.min(0.5),
                    });
                }
                for item in named {
                    let IrExpr::Object(pairs) = item else {
                        continue;
                    };
                    let (job, job_dynamic) = job_name(ctx, find_key(pairs, "name"));
                    out.push(Producer {
                        queue: queue.clone(),
                        dynamic,
                        job,
                        method: "add_bulk",
                        opts: object_of(find_key(pairs, "opts")),
                        receiver_type: receiver_type.clone(),
                        confidence: if job_dynamic {
                            receiver_conf.min(0.5)
                        } else {
                            receiver_conf
                        },
                    });
                }
            }
            ("upsertJobScheduler", _) => {
                let template = match args.get(2) {
                    Some(IrExpr::Object(pairs)) => Some(pairs.as_slice()),
                    _ => None,
                };
                let (job, job_dynamic) = match template.and_then(|p| find_key(p, "name")) {
                    Some(name) => job_name(ctx, Some(name)),
                    None => job_name(ctx, args.first()),
                };
                let mut opts = object_of(template.and_then(|p| find_key(p, "opts")));
                opts.push(("repeat".to_owned(), IrExpr::Bool(true)));
                out.push(Producer {
                    queue: queue.clone(),
                    dynamic,
                    job,
                    method: "upsert_scheduler",
                    opts,
                    receiver_type: receiver_type.clone(),
                    confidence: if job_dynamic {
                        receiver_conf.min(0.5)
                    } else {
                        receiver_conf
                    },
                });
            }
            _ => {}
        }
        let owner = ctx.owner(range.start);
        for producer in out {
            emit(ctx, owner, range, &producer);
        }
    }
}

fn object_of(expr: Option<&IrExpr>) -> Vec<(String, IrExpr)> {
    match expr {
        Some(IrExpr::Object(pairs)) => pairs.clone(),
        _ => Vec::new(),
    }
}

fn job_name(ctx: &FrameworkCtx<'_>, expr: Option<&IrExpr>) -> (String, bool) {
    match expr {
        Some(e) => match ctx.const_str(e) {
            Some(name) => (name, false),
            None => (format!("{{{}}}", expr_text(e)), true),
        },
        None => ("{unknown}".to_owned(), true),
    }
}

/// `FlowProducer.add({ name, queueName, children: [...] })`, children up to the depth cap.
fn flow(
    ctx: &FrameworkCtx<'_>,
    node: &IrExpr,
    depth: usize,
    receiver_type: &Option<String>,
    out: &mut Vec<Producer>,
) {
    let IrExpr::Object(pairs) = node else {
        return;
    };
    let (queue, dynamic, queue_conf) = queue_name(ctx, find_key(pairs, "queueName"));
    let (job, job_dynamic) = job_name(ctx, find_key(pairs, "name"));
    out.push(Producer {
        queue,
        dynamic,
        job,
        method: "flow_add",
        opts: object_of(find_key(pairs, "opts")),
        receiver_type: receiver_type.clone(),
        confidence: if job_dynamic {
            queue_conf.min(0.5)
        } else {
            queue_conf.min(0.9)
        },
    });
    if depth >= MAX_FLOW_DEPTH {
        return;
    }
    if let Some(IrExpr::Array(children)) = find_key(pairs, "children") {
        for child in children {
            flow(ctx, child, depth + 1, receiver_type, out);
        }
    }
}

/// Variables assigned from `new Queue('name')`.
fn queue_vars(ctx: &FrameworkCtx<'_>) -> BTreeMap<String, IrExpr> {
    let mut out = BTreeMap::new();
    for declarator in nodes_of_kinds(ctx.root, &[kind::VARIABLE_DECLARATOR]) {
        let (Some(name), Some(value)) = (
            declarator.child_by_field_name(field::NAME),
            declarator.child_by_field_name(field::VALUE),
        ) else {
            continue;
        };
        if name.kind() != kind::IDENTIFIER || value.kind() != kind::NEW_EXPRESSION {
            continue;
        }
        let Some(constructor) = value.child_by_field_name(field::CONSTRUCTOR) else {
            continue;
        };
        let ctor = ctx.src.text(constructor).into_owned();
        if ctor.rsplit('.').next() != Some("Queue") || origin(ctx, &ctor, "Queue").is_none() {
            continue;
        }
        if let Some(first) = arguments_of(value).into_iter().next() {
            out.insert(ctx.src.text(name).into_owned(), to_ir_expr(first, ctx.src));
        }
    }
    out
}

/// `(queue, dynamic, confidence, declared type, is flow producer)` when the receiver is a queue.
fn qualify(
    ctx: &FrameworkCtx<'_>,
    types: &BTreeMap<String, String>,
    injected: &InjectedMap,
    queue_vars: &BTreeMap<String, IrExpr>,
    receiver: &[String],
    range: SourceRange,
) -> Option<(String, bool, f32, Option<String>, bool)> {
    let (name, via_this) = match receiver {
        [this, prop] if this == "this" => (prop.as_str(), true),
        [var] => (var.as_str(), false),
        _ => return None,
    };
    let declared = types.get(name).cloned();
    let head = declared.as_deref().map(type_head).unwrap_or("");
    let is_flow = head == "FlowProducer";
    if via_this {
        let owner = ctx.owner(range.start);
        let class = ctx.class_of(owner)?;
        if let Some(token) = injected.get(&(class.local_id, name.to_owned())) {
            if token.kind == Some(TokenKind::BullmqQueue) {
                let (queue, dynamic, conf) = queue_name(ctx, token.queue.as_ref());
                return Some((queue, dynamic, conf.min(0.9), declared, false));
            }
        }
    } else if let Some(expr) = queue_vars.get(name) {
        let (queue, dynamic, conf) = queue_name(ctx, Some(expr));
        return Some((queue, dynamic, conf.min(0.9), declared, false));
    }
    if head == "Queue" || is_flow {
        if is_flow {
            return Some((String::new(), false, 0.9, declared, true));
        }
        return Some((format!("{{{name}}}"), true, 0.5, declared, false));
    }
    None
}

fn emit(ctx: &mut FrameworkCtx<'_>, owner: analysis_ir::LocalId, range: SourceRange, p: &Producer) {
    let flag = |key: &str| p.opts.iter().any(|(k, _)| k == key);
    let job_id = p.opts.iter().find(|(k, _)| k == "jobId").map(|(_, v)| v);
    let mut attrs = BTreeMap::new();
    attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
    attrs.insert("queue".to_owned(), AttrValue::Str(p.queue.clone()));
    if p.dynamic {
        attrs.insert("queue_dynamic".to_owned(), AttrValue::Bool(true));
    }
    attrs.insert("job".to_owned(), AttrValue::Str(p.job.clone()));
    attrs.insert("method".to_owned(), s(p.method));
    attrs.insert(
        "job_id_set".to_owned(),
        AttrValue::Bool(flag("jobId") || flag("deduplication")),
    );
    if let Some(expr) = job_id {
        attrs.insert("job_id_expr_kind".to_owned(), s(expr_kind(expr)));
    }
    attrs.insert("delay_set".to_owned(), AttrValue::Bool(flag("delay")));
    attrs.insert("repeat_set".to_owned(), AttrValue::Bool(flag("repeat")));
    if let Some(declared) = &p.receiver_type {
        attrs.insert(
            "receiver_declared_type".to_owned(),
            AttrValue::Str(declared.clone()),
        );
    }
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::QueueProducer,
        Some(owner),
        attrs,
        range,
        p.confidence,
    );
}
