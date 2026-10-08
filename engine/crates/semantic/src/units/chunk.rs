//! Code chunk units: function bodies split into overlapping line windows.

use review_core::ids::{RepositoryId, SnapshotId, SymbolKey};
use review_core::symbol::SymbolKind;

use super::symbol::SymbolInput;
use super::{derived_key, finish, EmbeddingUnit, UnitKind};

/// Lines per chunk.
pub const CHUNK_LINES: usize = 60;
/// Lines shared by consecutive chunks.
pub const CHUNK_OVERLAP: usize = 10;
/// Cap on chunk text.
pub const CHUNK_MAX_CHARS: usize = 6_000;
/// Bodies of at most this many lines are not chunked.
const MIN_BODY_LINES: usize = 3;

fn chunked(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function
            | SymbolKind::Method
            | SymbolKind::Constructor
            | SymbolKind::Getter
            | SymbolKind::Setter
    )
}

/// Key of chunk `ordinal` of a symbol: `blake3(symbol_key ‖ ordinal)`.
pub fn chunk_key(symbol_key: &SymbolKey, ordinal: usize) -> String {
    derived_key(
        "semantic-chunk",
        &[&symbol_key.to_string(), &ordinal.to_string()],
    )
}

/// `[start, end)` line windows of at most `CHUNK_LINES` with `CHUNK_OVERLAP` lines of overlap.
pub(crate) fn windows(n: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let step = CHUNK_LINES - CHUNK_OVERLAP;
    let mut start = 0;
    loop {
        let end = (start + CHUNK_LINES).min(n);
        out.push((start, end));
        if end == n {
            return out;
        }
        start += step;
    }
}

/// Chunk units of a function-like symbol's body. Empty for degraded parses (no body), bodies of
/// three lines or fewer, generated files and files with a secrets signal.
pub fn code_chunks(
    sym: &SymbolInput,
    repository_id: RepositoryId,
    snapshot_id: SnapshotId,
) -> Vec<EmbeddingUnit> {
    let Some(body) = sym.body.as_deref() else {
        return Vec::new();
    };
    if !chunked(sym.kind) || sym.is_generated || sym.has_secrets {
        return Vec::new();
    }
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() <= MIN_BODY_LINES {
        return Vec::new();
    }
    let header = sym
        .signature
        .as_deref()
        .and_then(|s| s.lines().next())
        .unwrap_or(&sym.qualified_name)
        .trim()
        .to_owned();
    let symbol_key = sym.symbol_key();
    windows(lines.len())
        .into_iter()
        .enumerate()
        .map(|(ordinal, (start, end))| {
            let key = chunk_key(&symbol_key, ordinal);
            let mut meta = sym.meta(UnitKind::CodeChunk, key, repository_id, snapshot_id);
            let first = sym.start_line.saturating_add(start as u32);
            meta.start_line = Some(first);
            meta.end_line = Some(sym.start_line.saturating_add(end as u32).saturating_sub(1));
            let text = format!("{header}\n{}", lines[start..end].join("\n"));
            finish(meta, &text, CHUNK_MAX_CHARS)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_cover_with_overlap() {
        assert_eq!(windows(0), vec![]);
        assert_eq!(windows(60), vec![(0, 60)]);
        assert_eq!(windows(61), vec![(0, 60), (50, 61)]);
        assert_eq!(windows(150), vec![(0, 60), (50, 110), (100, 150)]);
    }
}
