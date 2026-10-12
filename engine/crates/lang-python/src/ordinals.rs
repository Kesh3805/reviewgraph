//! Deterministic `~n` ordinals for symbols that share `(qualified name, kind)` (SID-003).
//!
//! Within a colliding group members are ordered by `(is_static, source start)`; the first keeps
//! ordinal 0 (no suffix) and the rest get 1, 2, ... Adding a duplicate after an existing symbol
//! therefore never changes the existing symbol's identity. Anonymous functions never have a bare
//! form: their ordinals start at 1.

use std::collections::BTreeMap;

use analysis_ir::{DiagCode, IrSymbol, Modifiers};
use review_core::symbol::SymbolKind;

const ANONYMOUS: &str = "<anonymous>";

/// Summary of one ordinal pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrdinalReport {
    /// Groups with more than one member (benign merges included).
    pub colliding_groups: usize,
    /// Groups that are real duplicates (not static/instance pairs or merged declarations).
    pub duplicate_groups: Vec<(Vec<String>, SymbolKind)>,
    /// Symbols dropped because a group exceeded `u16::MAX` members.
    pub dropped: usize,
    /// Codes of diagnostics the caller should record (one per kind of problem).
    pub diagnostics: Vec<DiagCode>,
}

type GroupKey = (Vec<String>, SymbolKind);

/// Assigns ordinals in place. Input order is irrelevant: groups are sorted by source offsets, so
/// the result is stable across runs and platforms. Idempotent.
pub fn assign_ordinals(symbols: &mut [IrSymbol]) -> OrdinalReport {
    let mut report = OrdinalReport::default();
    let mut groups: BTreeMap<GroupKey, Vec<usize>> = BTreeMap::new();
    for (index, symbol) in symbols.iter().enumerate() {
        groups
            .entry((symbol.qualified_name.clone(), symbol.kind))
            .or_default()
            .push(index);
    }
    for ((qualified_name, kind), mut members) in groups {
        let anonymous = qualified_name.last().map(String::as_str) == Some(ANONYMOUS);
        if members.len() == 1 && !anonymous {
            symbols[members[0]].ordinal = 0;
            continue;
        }
        members.sort_by_key(|&i| {
            let s = &symbols[i];
            (
                s.modifiers.contains(Modifiers::STATIC),
                s.range.start.line,
                s.range.start.column,
                i,
            )
        });
        if members.len() > 1 {
            report.colliding_groups += 1;
            let benign = kind == SymbolKind::Interface
                || kind == SymbolKind::Namespace
                || members
                    .iter()
                    .any(|&i| symbols[i].modifiers.contains(Modifiers::STATIC))
                    && members
                        .iter()
                        .any(|&i| !symbols[i].modifiers.contains(Modifiers::STATIC));
            if !benign && !anonymous {
                report.duplicate_groups.push((qualified_name.clone(), kind));
            }
        }
        let base: usize = usize::from(anonymous);
        for (position, &index) in members.iter().enumerate() {
            let ordinal = position + base;
            match u16::try_from(ordinal) {
                Ok(n) => symbols[index].ordinal = n,
                Err(_) => {
                    report.dropped += 1;
                    symbols[index].ordinal = u16::MAX;
                }
            }
        }
    }
    if !report.duplicate_groups.is_empty() {
        report.diagnostics.push(DiagCode::DuplicateSymbol);
    }
    report
}
