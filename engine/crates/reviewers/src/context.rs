//! The bounded context a reviewer consumes.
//!
//! **Stand-in for CTX-008.** The context engine (`context-engine`, CTX-*) does not yet expose a
//! `ContextPackage`, so reviewers are written against the minimal [`ReviewContext`] trait below.
//! [`ContextSnapshot`] is a plain, serialisable implementation used by tests and by the
//! evaluation harness (hand-authored packages in `benchmarks/quality/cases/*/context.json`).
//! When CTX-008 lands, `ContextPackage` implements this trait and nothing else changes.
//!
//! Item order is significant: refs (`S1`, `N1`, ...) are numbered in the order items appear, and
//! the context engine sorts items deterministically.

use serde::{Deserialize, Serialize};

/// A changed symbol in the cluster under review (`S#`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangedSymbol {
    /// Canonical `SymbolId` text.
    pub symbol_id: String,
    pub kind: String,
    pub path: String,
    /// Head-side line range `[start, end]`.
    pub range: [u32; 2],
    /// Base-side line range, when the symbol exists in the base.
    #[serde(default)]
    pub base_range: Option<[u32; 2]>,
    /// Number of lines of the head file, when known (anchor validation).
    #[serde(default)]
    pub file_lines: Option<u32>,
    #[serde(default)]
    pub change_classes: Vec<String>,
    #[serde(default)]
    pub signature_base: Option<String>,
    #[serde(default)]
    pub signature_head: Option<String>,
    #[serde(default)]
    pub body_base: Option<String>,
    #[serde(default)]
    pub body_head: Option<String>,
    #[serde(default)]
    pub hunks: Vec<Hunk>,
    /// The file is generated or vendored (INIT-008 / `generated.ignore`).
    #[serde(default)]
    pub generated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hunk {
    pub old: [u32; 2],
    pub new: [u32; 2],
}

/// A graph neighbour of the changed symbols (`N#`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeighborNode {
    pub symbol_id: String,
    pub kind: String,
    pub path: String,
    pub range: [u32; 2],
    #[serde(default)]
    pub file_lines: Option<u32>,
    pub distance: u32,
    #[serde(default)]
    pub excerpt: String,
}

/// A graph edge between two items, by `SymbolId`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeighborEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub confidence: f64,
}

/// A test that covers changed code (`T#`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestItem {
    pub test_id: String,
    pub path: String,
    /// `SymbolId`s the test covers.
    #[serde(default)]
    pub covers: Vec<String>,
}

/// A repository rule (`R#`). The text is untrusted data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleItem {
    pub rule_id: String,
    pub text: String,
    pub source: String,
}

/// A deterministic tool diagnostic (`D#`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticItem {
    pub tool: String,
    pub path: String,
    pub line: u32,
    pub code: String,
    pub message: String,
}

/// A configuration or schema item (`C#`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigItem {
    pub item_id: String,
    pub path: String,
    #[serde(default)]
    pub range: Option<[u32; 2]>,
    #[serde(default)]
    pub excerpt: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSummary {
    pub files_changed: u32,
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub change_classes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskSignal {
    pub signal: String,
    pub weight: f64,
}

/// What a reviewer reads. Implemented by [`ContextSnapshot`] today and by CTX-008's
/// `ContextPackage` later.
pub trait ReviewContext: Send + Sync {
    /// Stable hash of the package content (part of the reviewer `input_hash`).
    fn package_hash(&self) -> String;
    fn change_summary(&self) -> &ChangeSummary;
    fn changed_symbols(&self) -> &[ChangedSymbol];
    fn nodes(&self) -> &[NeighborNode];
    fn edges(&self) -> &[NeighborEdge];
    fn tests(&self) -> &[TestItem];
    fn rules(&self) -> &[RuleItem];
    fn diagnostics(&self) -> &[DiagnosticItem];
    fn config_items(&self) -> &[ConfigItem];
    fn risk_signals(&self) -> &[RiskSignal];
}

/// A serialisable [`ReviewContext`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSnapshot {
    #[serde(default)]
    pub change_summary: ChangeSummary,
    #[serde(default)]
    pub changed_symbols: Vec<ChangedSymbol>,
    #[serde(default)]
    pub nodes: Vec<NeighborNode>,
    #[serde(default)]
    pub edges: Vec<NeighborEdge>,
    #[serde(default)]
    pub tests: Vec<TestItem>,
    #[serde(default)]
    pub rules: Vec<RuleItem>,
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticItem>,
    #[serde(default)]
    pub config_items: Vec<ConfigItem>,
    #[serde(default)]
    pub risk_signals: Vec<RiskSignal>,
}

impl ReviewContext for ContextSnapshot {
    fn package_hash(&self) -> String {
        let value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        model_gateway::request_hash::hash_value(&value)
    }
    fn change_summary(&self) -> &ChangeSummary {
        &self.change_summary
    }
    fn changed_symbols(&self) -> &[ChangedSymbol] {
        &self.changed_symbols
    }
    fn nodes(&self) -> &[NeighborNode] {
        &self.nodes
    }
    fn edges(&self) -> &[NeighborEdge] {
        &self.edges
    }
    fn tests(&self) -> &[TestItem] {
        &self.tests
    }
    fn rules(&self) -> &[RuleItem] {
        &self.rules
    }
    fn diagnostics(&self) -> &[DiagnosticItem] {
        &self.diagnostics
    }
    fn config_items(&self) -> &[ConfigItem] {
        &self.config_items
    }
    fn risk_signals(&self) -> &[RiskSignal] {
        &self.risk_signals
    }
}
