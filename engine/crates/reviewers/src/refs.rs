//! Short refs (REV-001): the model cites `S1`, `N2`, `T1`, ... instead of paths and ids.
//!
//! | Prefix | Item |
//! |---|---|
//! | `S#` | changed symbol |
//! | `N#` | graph neighbourhood node |
//! | `T#` | test |
//! | `R#` | repository rule |
//! | `D#` | deterministic diagnostic |
//! | `C#` | configuration or schema item |
//!
//! Numbering follows the (already deterministic) order of the context items, starting at 1. The
//! [`RefTable`] maps every ref back to its target, path and ranges; paths shown to the model are
//! never read back from model output.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::context::ReviewContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    ChangedSymbol,
    Node,
    Test,
    Rule,
    Diagnostic,
    Config,
}

impl RefKind {
    pub const fn prefix(self) -> char {
        match self {
            Self::ChangedSymbol => 'S',
            Self::Node => 'N',
            Self::Test => 'T',
            Self::Rule => 'R',
            Self::Diagnostic => 'D',
            Self::Config => 'C',
        }
    }

    pub fn from_ref(r: &str) -> Option<Self> {
        let mut chars = r.chars();
        let kind = match chars.next()? {
            'S' => Self::ChangedSymbol,
            'N' => Self::Node,
            'T' => Self::Test,
            'R' => Self::Rule,
            'D' => Self::Diagnostic,
            'C' => Self::Config,
            _ => return None,
        };
        let digits = chars.as_str();
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(kind)
    }

    /// Symbol refs (`S#`, `N#`) resolve to a `SymbolId`.
    pub const fn is_symbol(self) -> bool {
        matches!(self, Self::ChangedSymbol | Self::Node)
    }
}

/// What a ref points to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefEntry {
    pub kind: RefKind,
    /// `SymbolId` text for symbol refs, otherwise the test, rule, diagnostic or config id.
    pub target: String,
    pub path: String,
    /// Head-side line range.
    pub range: Option<[u32; 2]>,
    /// Base-side line range (changed symbols that exist in the base).
    pub base_range: Option<[u32; 2]>,
    /// Number of lines in the head file, when known.
    pub file_lines: Option<u32>,
}

/// Ref → target, built once per reviewer call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RefTable {
    entries: BTreeMap<String, RefEntry>,
}

fn label(kind: RefKind, index: usize) -> String {
    format!("{}{}", kind.prefix(), index + 1)
}

impl RefTable {
    /// Numbers every context item in order.
    pub fn build(cx: &dyn ReviewContext) -> Self {
        let mut entries = BTreeMap::new();
        for (i, s) in cx.changed_symbols().iter().enumerate() {
            entries.insert(
                label(RefKind::ChangedSymbol, i),
                RefEntry {
                    kind: RefKind::ChangedSymbol,
                    target: s.symbol_id.clone(),
                    path: s.path.clone(),
                    range: Some(s.range),
                    base_range: s.base_range,
                    file_lines: s.file_lines,
                },
            );
        }
        for (i, n) in cx.nodes().iter().enumerate() {
            entries.insert(
                label(RefKind::Node, i),
                RefEntry {
                    kind: RefKind::Node,
                    target: n.symbol_id.clone(),
                    path: n.path.clone(),
                    range: Some(n.range),
                    base_range: None,
                    file_lines: n.file_lines,
                },
            );
        }
        for (i, t) in cx.tests().iter().enumerate() {
            entries.insert(
                label(RefKind::Test, i),
                RefEntry {
                    kind: RefKind::Test,
                    target: t.test_id.clone(),
                    path: t.path.clone(),
                    range: None,
                    base_range: None,
                    file_lines: None,
                },
            );
        }
        for (i, r) in cx.rules().iter().enumerate() {
            entries.insert(
                label(RefKind::Rule, i),
                RefEntry {
                    kind: RefKind::Rule,
                    target: r.rule_id.clone(),
                    path: String::new(),
                    range: None,
                    base_range: None,
                    file_lines: None,
                },
            );
        }
        for (i, d) in cx.diagnostics().iter().enumerate() {
            entries.insert(
                label(RefKind::Diagnostic, i),
                RefEntry {
                    kind: RefKind::Diagnostic,
                    target: format!("{}:{}", d.tool, d.code),
                    path: d.path.clone(),
                    range: Some([d.line, d.line]),
                    base_range: None,
                    file_lines: None,
                },
            );
        }
        for (i, c) in cx.config_items().iter().enumerate() {
            entries.insert(
                label(RefKind::Config, i),
                RefEntry {
                    kind: RefKind::Config,
                    target: c.item_id.clone(),
                    path: c.path.clone(),
                    range: c.range,
                    base_range: None,
                    file_lines: None,
                },
            );
        }
        Self { entries }
    }

    pub fn get(&self, r: &str) -> Option<&RefEntry> {
        self.entries.get(r)
    }

    pub fn contains(&self, r: &str) -> bool {
        self.entries.contains_key(r)
    }

    /// The first ref (in numbering order) whose target is `symbol_id`.
    pub fn ref_for_symbol(&self, symbol_id: &str) -> Option<String> {
        let mut found: Vec<&String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.kind.is_symbol() && e.target == symbol_id)
            .map(|(k, _)| k)
            .collect();
        found.sort_by_key(|k| (!k.starts_with('S'), k.len(), (*k).clone()));
        found.first().map(|k| (*k).clone())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &RefEntry)> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
