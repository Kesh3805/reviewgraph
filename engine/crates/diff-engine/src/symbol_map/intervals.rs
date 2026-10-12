//! Per-file "innermost symbol per line" index (DIFF-006).
//!
//! Symbols are painted onto a line table in `(start asc, end desc, local id)` order, so a nested
//! symbol always overwrites its container and every line ends up owned by its innermost symbol
//! (deepest, then earliest start). Decorator lines are painted last onto the decorated symbol.
//! Lines owned by nothing belong to the module. Building is `O(lines + Σ symbol extents)`; a
//! lookup is `O(1)`. Only changed files are indexed.

use analysis_ir::symbol::IrSymbol;
use analysis_ir::unit::ParsedUnit;
use review_core::location::SourceRange;
use review_core::symbol::SymbolKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which part of a symbol a changed line falls in. Ordered by precedence when one hit covers
/// several parts (`Body` wins).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HitScope {
    /// Outside every symbol (imports, top-level statements).
    ModuleLevel,
    /// A decorator applied to the symbol.
    Decorator,
    /// The symbol's own header: signature lines, a class's heritage, members without symbols.
    Header,
    /// The symbol's body.
    Body,
}

/// Upper bound on indexed lines when the side's line count is unknown.
pub const MAX_INDEX_LINES: u32 = 1_000_000;

/// Owner of one line: the symbol's local id and the scope of the line within it.
pub type Owner = Option<(u32, HitScope)>;

/// The line table of one unit.
#[derive(Debug, Clone, Default)]
pub struct LineIndex {
    /// `owners[line - 1]` for lines `1..=len`.
    owners: Vec<Owner>,
}

/// First and last (inclusive) line of a range; an end at column 0 belongs to the previous line.
pub fn line_span(range: &SourceRange) -> (u32, u32) {
    let start = range.start.line.max(1);
    let mut end = range.end.line.max(start);
    if range.end.column == 0 && end > start {
        end -= 1;
    }
    (start, end)
}

/// Whether a symbol kind is a container whose own lines are header lines.
pub fn is_container(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Enum
            | SymbolKind::Namespace
            | SymbolKind::Module
    )
}

/// Symbols that never own lines: the module (implicit owner of the rest) and parameters (their
/// lines belong to the enclosing callable).
fn indexed(symbol: &IrSymbol) -> bool {
    !matches!(symbol.kind, SymbolKind::Module | SymbolKind::Parameter)
}

impl LineIndex {
    /// Index `unit`. `lines` is the side's line count (lines beyond it are out of range).
    pub fn build(unit: &ParsedUnit, lines: u32) -> Self {
        // Symbol ranges never extend the table past the file (a hostile IR cannot make it huge).
        let max_line = if lines > 0 {
            lines
        } else {
            unit.symbols
                .iter()
                .filter(|s| indexed(s))
                .map(|s| line_span(&s.range).1)
                .max()
                .unwrap_or(0)
                .min(MAX_INDEX_LINES)
        };
        let mut owners: Vec<Owner> = vec![None; max_line as usize];
        let mut order: Vec<(u32, u32, u32)> = unit
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| indexed(s))
            .map(|(i, s)| {
                let (a, b) = line_span(&s.range);
                (a, b, i as u32)
            })
            .collect();
        order.sort_by(|x, y| x.0.cmp(&y.0).then(y.1.cmp(&x.1)).then(x.2.cmp(&y.2)));
        for &(start, end, local) in &order {
            let Some(symbol) = unit.symbols.get(local as usize) else {
                continue;
            };
            let body_start = symbol.body_range.as_ref().map(|r| line_span(r).0);
            for line in start..=end.min(max_line) {
                let scope = if is_container(symbol.kind) {
                    HitScope::Header
                } else {
                    match body_start {
                        Some(b) if line < b => HitScope::Header,
                        _ => HitScope::Body,
                    }
                };
                if let Some(slot) = owners.get_mut(line as usize - 1) {
                    *slot = Some((local, scope));
                }
            }
        }
        for &(_, _, local) in &order {
            let Some(symbol) = unit.symbols.get(local as usize) else {
                continue;
            };
            for decorator in &symbol.decorators {
                let (a, b) = line_span(&decorator.range);
                for line in a..=b.min(max_line) {
                    if let Some(slot) = owners.get_mut(line as usize - 1) {
                        *slot = Some((local, HitScope::Decorator));
                    }
                }
            }
        }
        Self { owners }
    }

    /// Number of indexed lines.
    pub fn len(&self) -> u32 {
        self.owners.len() as u32
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    /// Owner of `line` (1-based): `None` beyond the end, `Some(None)` for module-level lines.
    pub fn owner(&self, line: u32) -> Option<Owner> {
        if line == 0 {
            return None;
        }
        self.owners.get(line as usize - 1).copied()
    }
}
