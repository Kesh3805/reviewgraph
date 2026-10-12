//! Per-symbol syntax facts and source annotations the miners read alongside the graph.
//!
//! The graph carries structure; a few conventions need what happens *inside* a body (a database
//! write, a transaction wrapper, a thrown class, a job id). Those come from the analyzer's
//! [`SyntaxFact`]s (TSA-006), indexed here by the symbol's node key. The detail keys the miners
//! read are part of this contract:
//!
//! | Fact kind | Detail keys |
//! |---|---|
//! | `DbWriteLike` | `target` (optional) |
//! | `TransactionWrapper` | `kind` (optional); its range is the wrapped region |
//! | `Throw` | `class`: the thrown class name |
//! | `Call` | `callee`: the dotted callee text; for a job-producing call also `queue_add: true` and `job_id`: the `jobId` expression text when present |
//! | `GuardDecorator` | `guard`: the guard name |

use std::collections::HashMap;

use analysis_ir::facts::{FactKind, SyntaxFact};
use analysis_ir::symbol::AttrValue;
use codegraph::NodeKey;

/// `// review-ignore: <convention-id>` found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreAnnotation {
    pub path: String,
    /// 1-based line of the comment.
    pub line: u32,
    pub convention_id: String,
}

/// Syntax facts by symbol, plus the review-ignore annotations of the snapshot.
#[derive(Debug, Clone, Default)]
pub struct FactIndex {
    by_symbol: HashMap<NodeKey, Vec<SyntaxFact>>,
    annotations: Vec<IgnoreAnnotation>,
}

impl FactIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, symbol: NodeKey, facts: Vec<SyntaxFact>) {
        self.by_symbol.entry(symbol).or_default().extend(facts);
    }

    pub fn facts(&self, symbol: NodeKey) -> &[SyntaxFact] {
        self.by_symbol.get(&symbol).map_or(&[], Vec::as_slice)
    }

    pub fn facts_of_kind(&self, symbol: NodeKey, kind: FactKind) -> impl Iterator<Item = &SyntaxFact> {
        self.facts(symbol).iter().filter(move |f| f.kind == kind)
    }

    /// Every symbol's facts, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&NodeKey, &[SyntaxFact])> {
        self.by_symbol.iter().map(|(k, v)| (k, v.as_slice()))
    }

    pub fn add_annotation(&mut self, annotation: IgnoreAnnotation) {
        self.annotations.push(annotation);
    }

    /// Scans a file's text for `review-ignore: <id>` comments and records them.
    pub fn scan_review_ignore(&mut self, path: &str, text: &str) {
        self.annotations.extend(scan_review_ignore(path, text));
    }

    /// Whether `convention_id` is waived for a sample at `path:line`: an annotation on the same
    /// line or on one of the two lines above it.
    pub fn is_waived(&self, convention_id: &str, path: &str, line: u32) -> bool {
        self.annotations.iter().any(|a| {
            a.path == path
                && (a.convention_id == convention_id || a.convention_id == "*")
                && a.line <= line
                && line.saturating_sub(a.line) <= 2
        })
    }

    pub fn is_empty(&self) -> bool {
        self.by_symbol.is_empty() && self.annotations.is_empty()
    }
}

/// Every `review-ignore: <id>` comment in `text` (1-based lines).
pub fn scan_review_ignore(path: &str, text: &str) -> Vec<IgnoreAnnotation> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some(at) = line.find("review-ignore:") else {
            continue;
        };
        let before = &line[..at];
        if !(before.contains("//") || before.contains("/*") || before.contains('#')) {
            continue;
        }
        let rest = line[at + "review-ignore:".len()..].trim();
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '>' | '*'))
            .collect();
        if !id.is_empty() {
            out.push(IgnoreAnnotation {
                path: path.to_owned(),
                line: u32::try_from(i + 1).unwrap_or(u32::MAX),
                convention_id: id,
            });
        }
    }
    out
}

/// A string detail of a fact.
pub fn detail_str<'a>(fact: &'a SyntaxFact, key: &str) -> Option<&'a str> {
    match fact.detail.get(key)? {
        AttrValue::Str(s) => Some(s.as_str()),
        _ => None,
    }
}

/// A boolean detail of a fact (`false` when absent).
pub fn detail_flag(fact: &SyntaxFact, key: &str) -> bool {
    matches!(fact.detail.get(key), Some(AttrValue::Bool(true)))
}

/// 1-based start line of a fact.
pub fn fact_line(fact: &SyntaxFact) -> u32 {
    fact.range.start.line
}

/// Whether `inner` lies inside `outer` (by range).
pub fn contains(outer: &SyntaxFact, inner: &SyntaxFact) -> bool {
    outer.range.start <= inner.range.start && inner.range.end <= outer.range.end
}
