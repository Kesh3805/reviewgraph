//! `BullModule.registerQueue`, `new Queue(...)` and `new Worker(...)` registrations.

use std::collections::BTreeMap;

use analysis_ir::{AttrValue, FrameworkFactKind, IrExpr};
use review_core::location::SourceRange;

use super::{origin, queue_name, ADAPTER};
use crate::frameworks::util::{arguments_of, find_key, nodes_of_kinds, s, str_list};
use crate::frameworks::FrameworkCtx;
use crate::kinds::{field, kind};
use crate::visit::expr::to_ir_expr;
use crate::visit::fact_keys::callee_of;

pub fn collect(ctx: &mut FrameworkCtx<'_>) {
    for call in nodes_of_kinds(ctx.root, &[kind::CALL_EXPRESSION]) {
        let Some(function) = call.child_by_field_name(field::FUNCTION) else {
            continue;
        };
        let Some(callee) = callee_of(function, ctx.src) else {
            continue;
        };
        if !matches!(
            callee.name.as_str(),
            "registerQueue"
                | "registerQueueAsync"
                | "registerFlowProducer"
                | "registerFlowProducerAsync"
        ) {
            continue;
        }
        let [receiver] = callee.receiver.as_slice() else {
            continue;
        };
        let Some(conf) = origin(ctx, receiver, "BullModule") else {
            continue;
        };
        let via = if callee.name.contains("Flow") {
            "flow"
        } else {
            "register_queue"
        };
        let range = ctx.src.range(call);
        let mut options: Vec<IrExpr> = Vec::new();
        for arg in arguments_of(call) {
            match to_ir_expr(arg, ctx.src) {
                IrExpr::Array(items) => options.extend(items),
                other => options.push(other),
            }
        }
        for option in options {
            if let IrExpr::Object(pairs) = &option {
                emit(ctx, pairs, via, range, conf);
            }
        }
    }

    for new_expr in nodes_of_kinds(ctx.root, &[kind::NEW_EXPRESSION]) {
        let Some(constructor) = new_expr.child_by_field_name(field::CONSTRUCTOR) else {
            continue;
        };
        let name = ctx.src.text(constructor).into_owned();
        let (wanted, via) = match name.rsplit('.').next().unwrap_or("") {
            "Queue" => ("Queue", "new_queue"),
            "Worker" => ("Worker", "new_worker"),
            _ => continue,
        };
        let Some(conf) = origin(ctx, &name, wanted) else {
            continue;
        };
        let args: Vec<IrExpr> = arguments_of(new_expr)
            .into_iter()
            .map(|a| to_ir_expr(a, ctx.src))
            .collect();
        let mut pairs: Vec<(String, IrExpr)> = Vec::new();
        if let Some(first) = args.first() {
            pairs.push(("name".to_owned(), first.clone()));
        }
        let options_index = if via == "new_worker" { 2 } else { 1 };
        if let Some(IrExpr::Object(opts)) = args.get(options_index) {
            pairs.extend(opts.iter().cloned());
        }
        let range = ctx.src.range(new_expr);
        emit(ctx, &pairs, via, range, conf);
    }
}

fn emit(
    ctx: &mut FrameworkCtx<'_>,
    pairs: &[(String, IrExpr)],
    via: &str,
    range: SourceRange,
    conf: f32,
) {
    let (queue, dynamic, name_conf) = queue_name(ctx, find_key(pairs, "name"));
    let owner = ctx.owner(range.start);
    let mut keys: Vec<String> = pairs
        .iter()
        .map(|(k, _)| k.clone())
        .filter(|k| k != "name")
        .collect();
    keys.sort();
    keys.dedup();
    let default_job_id = match find_key(pairs, "defaultJobOptions") {
        Some(IrExpr::Object(opts)) => find_key(opts, "jobId").is_some(),
        _ => false,
    };
    let mut attrs = BTreeMap::new();
    attrs.insert("queue".to_owned(), AttrValue::Str(queue));
    attrs.insert("queue_dynamic".to_owned(), AttrValue::Bool(dynamic));
    attrs.insert("symbol".to_owned(), AttrValue::Str(ctx.qname(owner)));
    attrs.insert("via".to_owned(), s(via));
    attrs.insert("options_keys".to_owned(), str_list(keys));
    attrs.insert(
        "default_job_id_set".to_owned(),
        AttrValue::Bool(default_job_id),
    );
    ctx.emit(
        ADAPTER,
        FrameworkFactKind::QueueRegistration,
        Some(owner),
        attrs,
        range,
        conf.min(name_conf),
    );
}
