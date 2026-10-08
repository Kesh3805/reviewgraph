//! `ModelReviewInput` (REV-001, PRD §89): the structured, bounded input every reviewer sends.
//!
//! Sections are emitted in a fixed order, most stable first, so provider prompt caches hit:
//! `task` and `repository_rules` carry cache breakpoints. Every item is labelled with its short
//! ref (see [`crate::refs`]).

use std::sync::Arc;

use model_gateway::{InputSection, TaskType};
use serde_json::{json, Value};

use crate::context::ReviewContext;
use crate::focus::FocusProfile;
use crate::refs::{RefKind, RefTable};

/// Section names in send order.
pub const SECTION_ORDER: [&str; 8] = [
    "task",
    "repository_rules",
    "change_summary",
    "changed_symbols",
    "graph_neighborhood",
    "tests",
    "risk_signals",
    "deterministic_findings",
];

/// The input sections plus the ref table used to read the answer back.
#[derive(Debug, Clone)]
pub struct ModelReviewInput {
    pub sections: Vec<InputSection>,
    pub refs: Arc<RefTable>,
}

fn label(kind: RefKind, i: usize) -> String {
    format!("{}{}", kind.prefix(), i + 1)
}

/// Builds the input for `task` from a context. `extra` sections (reviewer specific, for example
/// security patterns) are appended after the common ones.
pub fn build_input(
    task: TaskType,
    focus: &[FocusProfile],
    cx: &dyn ReviewContext,
    extra: Vec<InputSection>,
) -> ModelReviewInput {
    let refs = RefTable::build(cx);
    let ref_of = |symbol_id: &str| refs.ref_for_symbol(symbol_id);

    let mut focus_names: Vec<&str> = focus.iter().map(|f| f.as_str()).collect();
    focus_names.sort_unstable();
    focus_names.dedup();

    let task_section = json!({ "task": task.as_str(), "focus_profiles": focus_names });

    let rules: Vec<Value> = cx
        .rules()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            json!({ "ref": label(RefKind::Rule, i), "rule_id": r.rule_id, "text": r.text, "source": r.source })
        })
        .collect();
    let rules_section = json!({
        "untrusted_data": true,
        "rules": rules,
    });

    let summary = cx.change_summary();
    let summary_section = json!({
        "files_changed": summary.files_changed,
        "intent": summary.intent,
        "change_classes": summary.change_classes,
    });

    let changed: Vec<Value> = cx
        .changed_symbols()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({
                "ref": label(RefKind::ChangedSymbol, i),
                "symbol_id": s.symbol_id,
                "kind": s.kind,
                "path": s.path,
                "range": s.range,
                "base_range": s.base_range,
                "change_classes": s.change_classes,
                "signature_base": s.signature_base,
                "signature_head": s.signature_head,
                "body_base": s.body_base,
                "body_head": s.body_head,
                "hunks": s.hunks.iter().map(|h| json!({"old": h.old, "new": h.new})).collect::<Vec<_>>(),
            })
        })
        .collect();

    let nodes: Vec<Value> = cx
        .nodes()
        .iter()
        .enumerate()
        .map(|(i, n)| {
            json!({
                "ref": label(RefKind::Node, i),
                "symbol_id": n.symbol_id,
                "kind": n.kind,
                "path": n.path,
                "range": n.range,
                "distance": n.distance,
                "excerpt": n.excerpt,
            })
        })
        .collect();
    let edges: Vec<Value> = cx
        .edges()
        .iter()
        .filter_map(|e| {
            let from = ref_of(&e.from)?;
            let to = ref_of(&e.to)?;
            Some(json!({ "from": from, "to": to, "kind": e.kind, "confidence": e.confidence }))
        })
        .collect();
    let config: Vec<Value> = cx
        .config_items()
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "ref": label(RefKind::Config, i),
                "item_id": c.item_id,
                "path": c.path,
                "range": c.range,
                "excerpt": c.excerpt,
            })
        })
        .collect();
    let graph_section = json!({ "nodes": nodes, "edges": edges, "config_items": config });

    let tests: Vec<Value> = cx
        .tests()
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let covers: Vec<String> = t.covers.iter().filter_map(|s| ref_of(s)).collect();
            json!({ "ref": label(RefKind::Test, i), "test_id": t.test_id, "path": t.path, "covers": covers })
        })
        .collect();

    let risk: Vec<Value> = cx
        .risk_signals()
        .iter()
        .map(|r| json!({ "signal": r.signal, "weight": r.weight }))
        .collect();

    let diagnostics: Vec<Value> = cx
        .diagnostics()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            json!({
                "ref": label(RefKind::Diagnostic, i),
                "tool": d.tool,
                "path": d.path,
                "line": d.line,
                "code": d.code,
                "message": d.message,
            })
        })
        .collect();

    let mut sections = vec![
        InputSection::new("task", task_section).with_cache_breakpoint(),
        InputSection::new("repository_rules", rules_section).with_cache_breakpoint(),
        InputSection::new("change_summary", summary_section),
        InputSection::new("changed_symbols", Value::Array(changed)),
        InputSection::new("graph_neighborhood", graph_section),
        InputSection::new("tests", Value::Array(tests)),
        InputSection::new("risk_signals", Value::Array(risk)),
        InputSection::new("deterministic_findings", Value::Array(diagnostics)),
    ];
    sections.extend(extra);
    ModelReviewInput {
        sections,
        refs: Arc::new(refs),
    }
}
