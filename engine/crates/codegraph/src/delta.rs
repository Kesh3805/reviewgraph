//! The in-memory change representation (CG-010).
//!
//! One [`GraphDelta`] is what incremental indexing produces, what storage writes and what
//! compaction reads. There is no second representation of "what changed": a PR head is a base
//! graph plus a delta applied through [`crate::overlay::GraphOverlay`], never a copy of the base.
//!
//! # Semantics
//!
//! * A node exists iff `(in base ∧ ∉ nodes_removed) ∨ ∈ nodes_added`. An added node with a key
//!   the base already has *replaces* the base node's data.
//! * An edge exists iff `(in base ∧ identity ∉ edges_removed ∧ both endpoints exist) ∨ ∈
//!   edges_added`. Overriding an edge therefore means tombstone + add of the same identity
//!   (clarification C5), never a silent replacement.
//! * A file listed in [`GraphDelta::files`] replaces its base file entry; `Deleted` removes it.
//! * `unresolved_replaced` replaces the base unresolved list of each listed path
//!   (clarification C6), which is what makes a delta able to *drop* a reference it once recorded.

use std::fmt;

use review_core::language::Language;
use review_core::location::{ContentHash, RepoPath};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::edge::{Edge, EdgeIdentity};
use crate::graph::{NodeInput, UnresolvedRef};
use crate::node_id::NodeKey;

/// What happened to one file in this delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    /// Not in the base snapshot.
    Added,
    /// Content hash changed.
    Modified,
    /// In the base snapshot, gone now.
    Deleted,
    /// Same content, different path.
    Renamed { from: RepoPath },
    /// Content unchanged, but its references were re-resolved (its dependencies moved).
    Relinked,
}

impl FileChangeKind {
    /// Lower-snake label, used in metrics and in the API payload.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
            Self::Renamed { .. } => "renamed",
            Self::Relinked => "relinked",
        }
    }

    /// True when the delta may legitimately contribute nodes for this file.
    #[must_use]
    pub fn carries_nodes(&self) -> bool {
        !matches!(self, Self::Deleted)
    }
}

/// One file's entry in a delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileChange {
    pub path: RepoPath,
    pub change: FileChangeKind,
    /// `file_versions.id` once persisted.
    pub file_version_id: Option<i64>,
    pub content_hash: Option<ContentHash>,
    pub language: Option<Language>,
}

impl FileChange {
    /// A `Modified` change with only a new content hash.
    #[must_use]
    pub fn modified(path: RepoPath, content_hash: ContentHash) -> Self {
        Self {
            path,
            change: FileChangeKind::Modified,
            file_version_id: None,
            content_hash: Some(content_hash),
            language: None,
        }
    }

    /// A `Deleted` change.
    #[must_use]
    pub fn deleted(path: RepoPath) -> Self {
        Self {
            path,
            change: FileChangeKind::Deleted,
            file_version_id: None,
            content_hash: None,
            language: None,
        }
    }
}

/// How a symbol survived an edit (SID-005, ADR-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineageTransition {
    /// The key is unchanged, so nothing has to be re-keyed downstream.
    Unchanged,
    /// Same qualified name, different path.
    Moved { from: RepoPath, to: RepoPath },
    /// Different name, same body above the similarity threshold.
    Renamed { from: String, to: String },
    /// Same name, changed body.
    SignatureChanged,
    /// The declaration is gone and another one took its place.
    Replaced,
}

/// One lineage record: a symbol's identity transition between the base and the head.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LineageRecord {
    /// Key in the base snapshot, when the symbol existed there.
    pub from: Option<NodeKey>,
    /// Key in the head snapshot.
    pub to: NodeKey,
    pub transition: LineageTransition,
    /// Token-Jaccard similarity of the two bodies; `1.0` for an unchanged body.
    pub similarity: f32,
}

impl LineageRecord {
    /// An unchanged symbol.
    #[must_use]
    pub fn unchanged(key: NodeKey) -> Self {
        Self {
            from: Some(key),
            to: key,
            transition: LineageTransition::Unchanged,
            similarity: 1.0,
        }
    }
}

/// The complete set of changes that turn a base graph into a head graph.
///
/// Serializable because the wire codec persists deltas verbatim (CG-011). Deliberately *not*
/// `JsonSchema`: a delta is an internal hand-off, not part of the API contract, and the API
/// contract is [`crate::GraphOverlay`]'s query surface instead.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphDelta {
    /// The base graph's `SCHEMA_VERSION`. A mismatch is
    /// [`crate::overlay::OverlayError::SchemaVersionMismatch`].
    pub base_schema_version: u32,
    /// Sorted by path.
    pub files: Vec<FileChange>,
    /// Added nodes, and nodes whose data changed (same key replaces).
    pub nodes_added: Vec<NodeInput>,
    /// Keys whose nodes are gone.
    pub nodes_removed: Vec<NodeKey>,
    /// Added edges, including overrides (tombstone + add, clarification C5).
    pub edges_added: Vec<Edge>,
    /// Edge tombstones, by identity.
    pub edges_removed: Vec<EdgeIdentity>,
    /// Per-file replacement of the unresolved-reference list (clarification C6).
    pub unresolved_replaced: Vec<(RepoPath, Vec<UnresolvedRef>)>,
    /// Symbol identity transitions, for finding dedup and embedding re-keying.
    pub lineage: Vec<LineageRecord>,
}

impl GraphDelta {
    /// An empty delta over a base of `base_schema_version`.
    #[must_use]
    pub fn new(base_schema_version: u32) -> Self {
        Self {
            base_schema_version,
            ..Self::default()
        }
    }

    /// Sorts every list into the canonical order the overlay and the codec rely on, so two
    /// deltas describing the same change are identical.
    pub fn normalize(&mut self) {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        self.nodes_added.sort_by_key(|a| a.id.key());
        self.nodes_added.dedup_by(|a, b| a.id == b.id);
        self.nodes_removed.sort();
        self.nodes_removed.dedup();
        self.edges_added.sort();
        self.edges_added.dedup();
        self.edges_removed.sort();
        self.edges_removed.dedup();
        self.unresolved_replaced.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, refs) in &mut self.unresolved_replaced {
            refs.sort_by(|a, b| a.ordinal.cmp(&b.ordinal).then_with(|| a.name.cmp(&b.name)));
            refs.dedup();
        }
        self.lineage.sort_by_key(|a| a.to);
    }

    /// Does `delta` change anything at all?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.nodes_added.is_empty()
            && self.nodes_removed.is_empty()
            && self.edges_added.is_empty()
            && self.edges_removed.is_empty()
            && self.unresolved_replaced.is_empty()
    }

    /// The paths this delta touches, sorted.
    #[must_use]
    pub fn touched_files(&self) -> Vec<RepoPath> {
        let mut paths: Vec<RepoPath> = self
            .files
            .iter()
            .map(|change| change.path.clone())
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// The change recorded for `path`, if any.
    #[must_use]
    pub fn file(&self, path: &RepoPath) -> Option<&FileChange> {
        self.files.iter().find(|change| &change.path == path)
    }
}

impl fmt::Display for FileChangeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Renamed { from } => write!(f, "renamed from {}", from.as_str()),
            other => f.write_str(other.as_str()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::node_id::NodeId;

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).unwrap()
    }

    fn node(id: &str) -> NodeInput {
        NodeInput::new(NodeId::from_canonical(id), crate::NodeKind::Function, "f")
    }

    #[test]
    fn change_kinds_describe_what_a_delta_may_contribute() {
        assert_eq!(FileChangeKind::Added.as_str(), "added");
        assert_eq!(FileChangeKind::Relinked.as_str(), "relinked");
        assert!(FileChangeKind::Added.carries_nodes());
        assert!(FileChangeKind::Modified.carries_nodes());
        assert!(FileChangeKind::Relinked.carries_nodes());
        assert!(!FileChangeKind::Deleted.carries_nodes());
        let renamed = FileChangeKind::Renamed { from: path("a.ts") };
        assert!(renamed.carries_nodes());
        assert_eq!(renamed.to_string(), "renamed from a.ts");
    }

    #[test]
    fn file_change_constructors_carry_the_obvious_fields() {
        let hash = ContentHash::of(b"new");
        let modified = FileChange::modified(path("src/a.ts"), hash);
        assert_eq!(modified.change, FileChangeKind::Modified);
        assert_eq!(modified.content_hash, Some(hash));
        assert_eq!(modified.file_version_id, None);
        assert_eq!(modified.language, None);

        let deleted = FileChange::deleted(path("src/a.ts"));
        assert_eq!(deleted.change, FileChangeKind::Deleted);
        assert_eq!(deleted.content_hash, None);
    }

    #[test]
    fn normalize_is_idempotent_and_order_independent() {
        let mut one = GraphDelta::new(crate::SCHEMA_VERSION);
        one.files.push(FileChange::deleted(path("src/b.ts")));
        one.files.push(FileChange::modified(
            path("src/a.ts"),
            ContentHash::of(b"a"),
        ));
        one.nodes_added.push(node("ts:src/b.ts#B/f/function"));
        one.nodes_added.push(node("ts:src/a.ts#A/f/function"));
        one.nodes_removed
            .push(node("ts:src/c.ts#C/f/function").id.key());
        one.normalize();
        let snapshot = one.clone();

        let mut two = GraphDelta::new(crate::SCHEMA_VERSION);
        two.nodes_removed = one.nodes_removed.clone();
        two.nodes_added = one.nodes_added.clone();
        two.nodes_added.reverse();
        two.files = one.files.clone();
        two.files.reverse();
        two.normalize();

        assert_eq!(one, two);
        assert_eq!(one, snapshot);
        assert_eq!(
            one.touched_files(),
            vec![path("src/a.ts"), path("src/b.ts")]
        );
        assert!(one.file(&path("src/a.ts")).is_some());
        assert!(one.file(&path("src/z.ts")).is_none());
    }

    #[test]
    fn an_empty_delta_touches_nothing() {
        let delta = GraphDelta::new(crate::SCHEMA_VERSION);
        assert!(delta.is_empty());
        assert!(delta.touched_files().is_empty());
        assert!(!GraphDelta {
            files: vec![FileChange::deleted(path("a.ts"))],
            ..GraphDelta::new(crate::SCHEMA_VERSION)
        }
        .is_empty());
        assert_eq!(
            GraphDelta::new(crate::SCHEMA_VERSION).base_schema_version,
            crate::SCHEMA_VERSION
        );
    }

    #[test]
    fn unchanged_lineage_needs_no_rekeying() {
        let key = node("ts:src/a.ts#A/f/function").id.key();
        let record = LineageRecord::unchanged(key);
        assert_eq!(record.from, Some(key));
        assert_eq!(record.to, key);
        assert_eq!(record.transition, LineageTransition::Unchanged);
        assert_eq!(record.similarity, 1.0);
    }
}
