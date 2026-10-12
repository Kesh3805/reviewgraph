//! Embedding units (SEM-006): what gets embedded, with stable keys and content hashes.
//!
//! Builders are pure functions over thin input types ([`SymbolInput`], [`DocInput`],
//! [`ConventionInput`]) that the composition root fills from the IR, the code graph and the
//! repository profile. This keeps `semantic` independent of the analyzer crates.

mod chunk;
mod convention;
mod doc;
mod symbol;

use std::fmt;
use std::str::FromStr;

use review_core::ids::{RepositoryId, SnapshotId, SymbolKey};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::symbol::Hash128;
use serde::{Deserialize, Serialize};

pub use chunk::{chunk_key, code_chunks, CHUNK_LINES, CHUNK_MAX_CHARS, CHUNK_OVERLAP};
pub use convention::{convention_unit, ConventionInput};
pub use doc::{doc_units, DocInput, DOC_MAX_CHARS};
pub use symbol::{symbol_summary, SymbolInput, SUMMARY_MAX_CHARS};

use crate::error::Error;
use crate::metrics;
use crate::redact::redact;

/// Bumping this changes every content hash: a deliberate full re-embed.
pub const UNIT_TEMPLATE_VERSION: u16 = 1;

/// Kinds of embedded unit (ADR-008; `finding_history` is post-MVP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    SymbolSummary,
    CodeChunk,
    Doc,
    Convention,
}

impl UnitKind {
    pub const ALL: [UnitKind; 4] = [
        Self::SymbolSummary,
        Self::CodeChunk,
        Self::Doc,
        Self::Convention,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SymbolSummary => "symbol_summary",
            Self::CodeChunk => "code_chunk",
            Self::Doc => "doc",
            Self::Convention => "convention",
        }
    }
}

impl fmt::Display for UnitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for UnitKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| Error::InvalidInput(format!("unknown unit kind {s:?}")))
    }
}

/// One text to embed, with its metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingUnit {
    pub kind: UnitKind,
    /// Symbol key (hex) for summaries; chunk key for chunks and docs; convention id.
    pub key: String,
    pub repository_id: RepositoryId,
    pub language: Option<Language>,
    pub module: Option<String>,
    pub file_path: Option<RepoPath>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    /// The symbol a summary or chunk belongs to.
    pub symbol_key: Option<SymbolKey>,
    pub text: String,
    /// `blake3(kind ‖ template_version ‖ text)`; excludes the snapshot.
    pub content_hash: Hash128,
    pub snapshot_id: SnapshotId,
}

/// Content hash of a unit text under a template version.
pub fn content_hash(kind: UnitKind, template_version: u16, text: &str) -> Hash128 {
    let mut bytes = Vec::with_capacity(text.len() + 24);
    bytes.extend_from_slice(kind.as_str().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&template_version.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(text.as_bytes());
    Hash128::of("semantic-unit", &bytes)
}

/// Truncates to at most `max` characters on a char boundary.
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_owned(),
        None => s.to_owned(),
    }
}

/// Where a unit comes from; shared by the builders.
#[derive(Debug, Clone)]
pub(crate) struct UnitMeta {
    pub kind: UnitKind,
    pub key: String,
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub language: Option<Language>,
    pub module: Option<String>,
    pub file_path: Option<RepoPath>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub symbol_key: Option<SymbolKey>,
}

/// Redacts, truncates, hashes and counts. Every builder ends here.
pub(crate) fn finish(meta: UnitMeta, raw_text: &str, max_chars: usize) -> EmbeddingUnit {
    let text = truncate_chars(&redact(raw_text), max_chars);
    metrics::unit_built(meta.kind.as_str(), text.chars().count());
    EmbeddingUnit {
        content_hash: content_hash(meta.kind, UNIT_TEMPLATE_VERSION, &text),
        kind: meta.kind,
        key: meta.key,
        repository_id: meta.repository_id,
        language: meta.language,
        module: meta.module,
        file_path: meta.file_path,
        start_line: meta.start_line,
        end_line: meta.end_line,
        symbol_key: meta.symbol_key,
        text,
        snapshot_id: meta.snapshot_id,
    }
}

/// `blake3(parts joined by NUL)[..16]` as 32 hex characters.
pub(crate) fn derived_key(domain: &str, parts: &[&str]) -> String {
    Hash128::of(domain, parts.join("\0").as_bytes()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_char_boundaries() {
        assert_eq!(truncate_chars("héllo", 2), "hé");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }

    #[test]
    fn kinds_roundtrip() {
        for k in UnitKind::ALL {
            assert_eq!(k.as_str().parse::<UnitKind>().unwrap(), k);
        }
    }
}
