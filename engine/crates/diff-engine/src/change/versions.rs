//! [`SymbolVersion`]: one side of a changed symbol.

use analysis_ir::unit::ParsedUnit;
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::{RepoPath, SourceRange};
use review_core::symbol::{Hash128, SymbolKind};
use serde::Serialize;

/// One side of a changed symbol. Carries no source text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SymbolVersion {
    /// Storage key of `id`.
    pub key: SymbolKey,
    /// Symbol id on this side.
    pub id: SymbolId,
    /// File on this side.
    pub path: RepoPath,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Full range.
    pub range: SourceRange,
    /// Body range, if the symbol has a body.
    pub body_range: Option<SourceRange>,
    /// Display signature (at most 512 characters).
    pub signature: Option<String>,
    /// Signature hash.
    pub signature_hash: Hash128,
    /// Body hash.
    pub body_hash: Hash128,
    /// Attribute hash (decorators, modifiers, heritage).
    pub attr_hash: Hash128,
    /// Index of the symbol in its unit.
    pub local: u32,
}

impl SymbolVersion {
    /// The version of `local` in `unit`, or `None` when the unit has no such symbol.
    pub fn of(unit: &ParsedUnit, id: &SymbolId, path: &RepoPath, local: u32) -> Option<Self> {
        let s = unit.symbols.get(local as usize)?;
        Some(Self {
            key: SymbolKey::of(id),
            id: id.clone(),
            path: path.clone(),
            kind: s.kind,
            range: s.range,
            body_range: s.body_range,
            signature: s.signature.clone(),
            signature_hash: s.signature_hash,
            body_hash: s.body_hash,
            attr_hash: s.attr_hash,
            local,
        })
    }
}
