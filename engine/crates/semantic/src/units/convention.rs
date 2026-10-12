//! Convention units: one per inferred or declared repository convention (PROF-003).

use review_core::ids::{RepositoryId, SnapshotId};

use super::{finish, EmbeddingUnit, UnitKind, UnitMeta};

const CONVENTION_MAX_CHARS: usize = 1_000;

/// A convention from the repository profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ConventionInput {
    /// Stable convention id; becomes the unit key.
    pub id: String,
    pub rule: String,
    pub scope: String,
    /// Sample symbol names; the first two are used.
    pub examples: Vec<String>,
    pub confidence: f32,
}

/// `"{rule}\nscope: {scope}\nexamples: {2 names}\nconfidence: {c}"`.
pub fn convention_unit(
    c: &ConventionInput,
    repository_id: RepositoryId,
    snapshot_id: SnapshotId,
) -> EmbeddingUnit {
    let examples: Vec<&str> = c.examples.iter().take(2).map(String::as_str).collect();
    let examples = if examples.is_empty() {
        "-".to_owned()
    } else {
        examples.join(", ")
    };
    let text = format!(
        "{}\nscope: {}\nexamples: {examples}\nconfidence: {:.2}",
        c.rule.trim(),
        c.scope.trim(),
        c.confidence
    );
    let meta = UnitMeta {
        kind: UnitKind::Convention,
        key: c.id.clone(),
        repository_id,
        snapshot_id,
        language: None,
        module: None,
        file_path: None,
        start_line: None,
        end_line: None,
        symbol_key: None,
    };
    finish(meta, &text, CONVENTION_MAX_CHARS)
}
