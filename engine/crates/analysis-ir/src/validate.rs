//! Structural validation of a `ParsedUnit`.
//!
//! Runs in debug builds after every `analyze` and in all tests. It returns every violation, not
//! just the first.

use std::collections::BTreeSet;

use review_core::location::SourceRange;
use review_core::symbol::SymbolKind;

use crate::unit::ParsedUnit;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IrViolation {
    #[error("symbols[0] must be the module symbol")]
    FirstSymbolNotModule,
    #[error("symbol at index {index} has local_id {local_id}")]
    LocalIdMismatch { index: usize, local_id: u32 },
    #[error("{what} refers to local id {id}, but there are only {len} symbols")]
    DanglingLocalId {
        what: &'static str,
        id: u32,
        len: usize,
    },
    #[error("symbol {symbol} has parent {parent}, which does not precede it")]
    ParentDoesNotPrecede { symbol: u32, parent: u32 },
    #[error("symbol {symbol} is not contained in its parent {parent}")]
    ChildOutsideParent { symbol: u32, parent: u32 },
    #[error("{what} range ends beyond the last line ({lines})")]
    RangeBeyondFile { what: &'static str, lines: u32 },
    #[error("symbol facts are not sorted and unique by symbol")]
    FactsNotSorted,
    #[error("symbols {first} and {second} share (qualified_name, kind, ordinal)")]
    DuplicateIdentity { first: u32, second: u32 },
}

fn contains(outer: &SourceRange, inner: &SourceRange) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// Checks the invariants documented on [`ParsedUnit`]. `Ok` when there are none violated.
pub fn validate(unit: &ParsedUnit) -> Result<(), Vec<IrViolation>> {
    let mut out = Vec::new();
    let len = unit.symbols.len();
    match unit.symbols.first() {
        Some(first) if first.kind == SymbolKind::Module => {}
        _ => out.push(IrViolation::FirstSymbolNotModule),
    }
    for (index, symbol) in unit.symbols.iter().enumerate() {
        if symbol.local_id.0 as usize != index {
            out.push(IrViolation::LocalIdMismatch {
                index,
                local_id: symbol.local_id.0,
            });
        }
        if symbol.range.end.line > unit.stats.lines.max(1) {
            out.push(IrViolation::RangeBeyondFile {
                what: "symbol",
                lines: unit.stats.lines,
            });
        }
        if let Some(parent) = symbol.parent {
            if parent.0 as usize >= len {
                out.push(IrViolation::DanglingLocalId {
                    what: "symbol parent",
                    id: parent.0,
                    len,
                });
            } else if parent.0 as usize >= index {
                out.push(IrViolation::ParentDoesNotPrecede {
                    symbol: symbol.local_id.0,
                    parent: parent.0,
                });
            } else if !contains(&unit.symbols[parent.0 as usize].range, &symbol.range) {
                out.push(IrViolation::ChildOutsideParent {
                    symbol: symbol.local_id.0,
                    parent: parent.0,
                });
            }
        }
    }
    for reference in &unit.references {
        if reference.from.0 as usize >= len {
            out.push(IrViolation::DanglingLocalId {
                what: "reference",
                id: reference.from.0,
                len,
            });
        }
        if reference.range.end.line > unit.stats.lines.max(1) {
            out.push(IrViolation::RangeBeyondFile {
                what: "reference",
                lines: unit.stats.lines,
            });
        }
    }
    for export in &unit.exports {
        use crate::module::IrExport;
        let id = match export {
            IrExport::Local { symbol, .. } => Some(symbol.0),
            IrExport::CjsModuleExports { symbol, .. }
            | IrExport::CjsExportsProperty { symbol, .. }
            | IrExport::ExportAssignment { symbol } => symbol.map(|s| s.0),
            _ => None,
        };
        if let Some(id) = id.filter(|id| *id as usize >= len) {
            out.push(IrViolation::DanglingLocalId {
                what: "export",
                id,
                len,
            });
        }
    }
    for fact in &unit.framework {
        if let Some(symbol) = fact.symbol.filter(|s| s.0 as usize >= len) {
            out.push(IrViolation::DanglingLocalId {
                what: "framework fact",
                id: symbol.0,
                len,
            });
        }
    }
    let mut previous: Option<u32> = None;
    for group in &unit.facts {
        if group.symbol.0 as usize >= len {
            out.push(IrViolation::DanglingLocalId {
                what: "symbol facts",
                id: group.symbol.0,
                len,
            });
        }
        if previous.is_some_and(|p| p >= group.symbol.0) {
            out.push(IrViolation::FactsNotSorted);
        }
        previous = Some(group.symbol.0);
    }
    type Identity<'a> = (&'a [String], SymbolKind, u16);
    let mut seen: BTreeSet<Identity<'_>> = BTreeSet::new();
    let mut first_of: Vec<(Identity<'_>, u32)> = Vec::new();
    for symbol in &unit.symbols {
        let key = (
            symbol.qualified_name.as_slice(),
            symbol.kind,
            symbol.ordinal,
        );
        if !seen.insert(key) {
            let first = first_of
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, id)| *id)
                .unwrap_or(0);
            out.push(IrViolation::DuplicateIdentity {
                first,
                second: symbol.local_id.0,
            });
        } else {
            first_of.push((key, symbol.local_id.0));
        }
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}
