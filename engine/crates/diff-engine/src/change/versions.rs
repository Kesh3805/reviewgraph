//! One side's version of a changed symbol (CHG-001).

use analysis_ir::identity::symbol_id_of;
use analysis_ir::unit::ParsedUnit;
use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::{RepoPath, SourceRange};
use review_core::symbol::{Hash128, SymbolKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A symbol as it exists on one side of the diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolVersion {
    /// Storage key.
    pub key: SymbolKey,
    /// Canonical id.
    pub id: SymbolId,
    /// File on this side.
    pub path: RepoPath,
    /// Kind.
    pub kind: SymbolKind,
    /// Full range.
    pub range: SourceRange,
    /// Body range, when the symbol has a body.
    pub body_range: Option<SourceRange>,
    /// Display signature (at most 512 characters, bounded by the IR).
    pub signature: Option<String>,
    /// Signature hash.
    pub signature_hash: Hash128,
    /// Normalized body hash.
    pub body_hash: Hash128,
    /// Attribute (decorators, modifiers) hash.
    pub attr_hash: Hash128,
    /// Index of the symbol in its unit (`LocalId`).
    pub local: u32,
}

impl SymbolVersion {
    /// The version of `local` in `unit`.
    pub fn of(unit: &ParsedUnit, local: u32) -> Option<Self> {
        let symbol = unit.symbols.get(local as usize)?;
        let id = symbol_id_of(unit, local)?;
        Some(Self {
            key: SymbolKey::of(&id),
            id,
            path: unit.file.clone(),
            kind: symbol.kind,
            range: symbol.range,
            body_range: symbol.body_range,
            signature: symbol.signature.clone(),
            signature_hash: symbol.signature_hash,
            body_hash: symbol.body_hash,
            attr_hash: symbol.attr_hash,
            local,
        })
    }
}
