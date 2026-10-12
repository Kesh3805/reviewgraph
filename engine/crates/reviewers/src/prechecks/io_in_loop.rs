//! REV-P-002: I/O and queries inside loops.
//!
//! **Stand-in for syntax facts.** `SyntaxFact::Loop` does not reach this crate yet, so loops and
//! calls are located lexically in the head body of each changed symbol. Only loops whose lines
//! intersect a diff hunk are reported (an unchanged loop is not the change's fault). Calls that
//! reach I/O through up to two callees are followed through `CALLS` edges and neighbour excerpts.

use std::sync::LazyLock;

use regex::Regex;

use super::{DeterministicFinding, PrecheckOutcome, SeverityHint};
use crate::context::{ChangedSymbol, ReviewContext};

pub const RULE_IO_IN_LOOP: &str = "performance.io_in_loop";
pub const RULE_FAN_OUT: &str = "performance.fan_out_query";
/// Hops followed from a call inside a loop to an I/O primitive.
const MAX_HOPS: u32 = 2;

static LOOP: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\bfor\s*(await\s*)?\(|\bwhile\s*\(|\.(forEach|map|reduce)\(").ok()
});
static IO: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"await\s+(fetch\(|[\w.$]+\.(find\w*|save|insert|update|delete|remove|query|execute|get|post|put|patch|request|send)\()",
    )
    .ok()
});
static AWAIT_CALL: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"await\s+(?:this\.)?(?:[\w$]+\.)*([\w$]+)\(").ok());

fn hit(re: &LazyLock<Option<Regex>>, line: &str) -> bool {
    re.as_ref().is_some_and(|r| r.is_match(line))
}

/// Runs the detector over the changed symbols of `cx`.
pub fn io_in_loop_precheck(cx: &dyn ReviewContext) -> PrecheckOutcome {
    let mut findings = Vec::new();
    for s in cx.changed_symbols() {
        if s.generated {
            continue;
        }
        let Some(body) = s.body_head.as_deref() else {
            continue;
        };
        findings.extend(scan(cx, s, body));
    }
    PrecheckOutcome::Executed {
        findings,
        truncated: false,
    }
}

fn changed(s: &ChangedSymbol, line: u32) -> bool {
    s.hunks.is_empty()
        || s.hunks
            .iter()
            .any(|h| (h.new[0]..=h.new[1]).contains(&line))
}

/// End line index (inclusive) of the construct opened on `start`, by brace/paren balance.
fn extent(lines: &[&str], start: usize) -> usize {
    let mut depth: i32 = 0;
    let mut opened = false;
    for (i, l) in lines.iter().enumerate().skip(start) {
        for c in l.chars() {
            match c {
                '{' | '(' => {
                    depth += 1;
                    opened = true;
                }
                '}' | ')' => depth -= 1,
                _ => {}
            }
        }
        if opened && depth <= 0 {
            return i;
        }
    }
    lines.len().saturating_sub(1)
}

fn reaches_io(cx: &dyn ReviewContext, callee: &str, hops: u32) -> bool {
    if hops == 0 {
        return false;
    }
    cx.nodes()
        .iter()
        .filter(|n| {
            let name = n.symbol_id.rsplit(['.', '#']).next().unwrap_or("");
            name.split('/').next() == Some(callee)
        })
        .any(|n| {
            n.excerpt.lines().any(|l| hit(&IO, l))
                || cx
                    .edges()
                    .iter()
                    .filter(|e| e.kind == "CALLS" && e.from == n.symbol_id)
                    .any(|e| {
                        cx.nodes()
                            .iter()
                            .any(|m| m.symbol_id == e.to && m.excerpt.lines().any(|l| hit(&IO, l)))
                            || reaches_io(cx, callee_name(&e.to), hops - 1)
                    })
        })
}

fn callee_name(symbol_id: &str) -> &str {
    let tail = symbol_id.rsplit(['.', '#']).next().unwrap_or("");
    tail.split('/').next().unwrap_or(tail)
}

fn scan(cx: &dyn ReviewContext, s: &ChangedSymbol, body: &str) -> Vec<DeterministicFinding> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let head = lines[i];
        // A loop over a literal array is bounded by the source text.
        if !hit(&LOOP, head) || head.contains(" of [") {
            i += 1;
            continue;
        }
        let end = extent(&lines, i);
        let loop_line = s.range[0] + i as u32;
        let io = (i..=end).find(|&j| {
            let l = lines[j];
            if l.contains("In(") || l.contains(" IN (") {
                return false;
            }
            hit(&IO, l)
                || AWAIT_CALL
                    .as_ref()
                    .and_then(|r| r.captures(l))
                    .and_then(|c| c.get(1))
                    .is_some_and(|m| reaches_io(cx, m.as_str(), MAX_HOPS))
        });
        if let Some(j) = io {
            let io_line = s.range[0] + j as u32;
            if changed(s, loop_line) || changed(s, io_line) {
                let fan_out = head.contains("Promise.all") || head.contains(".map(async");
                let rule = if fan_out {
                    RULE_FAN_OUT
                } else {
                    RULE_IO_IN_LOOP
                };
                out.push(DeterministicFinding {
                    rule_id: rule.into(),
                    reviewer: "deterministic:performance".into(),
                    symbol_key: s.symbol_id.clone(),
                    path: s.path.clone(),
                    anchor_range: [loop_line, s.range[0] + end as u32],
                    severity_hint: if fan_out {
                        SeverityHint::Low
                    } else {
                        SeverityHint::Medium
                    },
                    message: format!(
                        "awaited I/O at line {io_line} runs once per iteration of the loop at line {loop_line}"
                    ),
                    absence: None,
                    members: vec![s.symbol_id.clone()],
                });
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
    out
}
