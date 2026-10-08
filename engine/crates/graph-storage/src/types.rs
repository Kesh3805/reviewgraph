//! Request and response types of the [`GraphStore`](crate::GraphStore) port (GS-001).
//!
//! Everything here is a plain value type: adapters own the encoding, callers own the lifecycle.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, OrganizationId, RepositoryId, SnapshotId};
use schemars::gen::SchemaGenerator;
use schemars::schema::{Schema, SchemaObject};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{Graph, GraphDelta, Node};
use crate::status::SnapshotStatus;

/// Version of the on-disk/on-row graph layout this crate reads and writes.
///
/// Mapping requirement: must stay equal to `codegraph::SCHEMA_VERSION` (CG-011). A snapshot
/// whose `graph_schema_version` differs is refused with `StoreError::SchemaMismatch` so the
/// caller rebuilds (INC-011).
pub const GRAPH_SCHEMA_VERSION: u32 = 1;

/// Whether a snapshot stands alone or extends a base (ADR-003).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotKind {
    Full,
    Delta,
}

impl SnapshotKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Delta => "delta",
        }
    }

    pub fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "full" => Some(Self::Full),
            "delta" => Some(Self::Delta),
            _ => None,
        }
    }
}

impl std::fmt::Display for SnapshotKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a snapshot exists. `compaction` marks GS-007 output.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotPurpose {
    DefaultBranch,
    PullRequest,
    Local,
    Compaction,
}

impl SnapshotPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DefaultBranch => "default_branch",
            Self::PullRequest => "pull_request",
            Self::Local => "local",
            Self::Compaction => "compaction",
        }
    }

    pub fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "default_branch" => Some(Self::DefaultBranch),
            "pull_request" => Some(Self::PullRequest),
            "local" => Some(Self::Local),
            "compaction" => Some(Self::Compaction),
            _ => None,
        }
    }
}

impl std::fmt::Display for SnapshotPurpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse outcome recorded on a file version (`file_versions.parse_status`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FileParseStatus {
    Ok,
    Partial,
    Failed,
    Skipped,
}

impl FileParseStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "ok" => Some(Self::Ok),
            "partial" => Some(Self::Partial),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// ADR-015 provenance recorded on every snapshot (`snapshots` version columns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotVersions {
    /// `codegraph::SCHEMA_VERSION`; a bump forces a full rebuild.
    pub graph_schema_version: u32,
    /// Analyzer and linker versions, sorted by name so the JSON hashes deterministically.
    pub analyzer_versions: BTreeMap<String, String>,
    /// Blake3 of the normalized configuration (32 bytes).
    pub config_hash: [u8; 32],
    /// Per-component hashes of the configuration, hex encoded.
    pub config_components: BTreeMap<String, String>,
    /// Repository fingerprint of PRD section 15 (ADR-015), 32 bytes.
    pub fingerprint: [u8; 32],
}

impl SnapshotVersions {
    /// Versions for a snapshot whose graph schema is [`GRAPH_SCHEMA_VERSION`].
    pub fn new(fingerprint: [u8; 32]) -> Self {
        Self {
            graph_schema_version: GRAPH_SCHEMA_VERSION,
            analyzer_versions: BTreeMap::new(),
            config_hash: [0u8; 32],
            config_components: BTreeMap::new(),
            fingerprint,
        }
    }
}

impl Default for SnapshotVersions {
    fn default() -> Self {
        Self::new([0u8; 32])
    }
}

/// Counters recorded when a snapshot is written. Churn for GS-007 comes from
/// `edges_added + edges_removed` accumulated over the chain, so no row counting happens at
/// decision time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SnapshotStats {
    pub files: u64,
    pub nodes: u64,
    pub edges: u64,
    pub unresolved_refs: u64,
    pub parse_errors: u64,
    /// Edges this snapshot added relative to its base.
    pub edges_added: u64,
    /// Tombstones this snapshot wrote relative to its base.
    pub edges_removed: u64,
    /// Set on a snapshot produced by `compact`.
    pub compacted_from: Option<SnapshotId>,
    /// The ids of the chain a compacted snapshot replaced.
    pub replaced_chain: Vec<SnapshotId>,
}

impl SnapshotStats {
    /// Counters implied by a full graph payload.
    pub fn from_graph(g: &Graph) -> Self {
        Self {
            files: g.files.len() as u64,
            nodes: g.nodes.len() as u64,
            edges: g.edges.len() as u64,
            unresolved_refs: g.unresolved.len() as u64,
            parse_errors: 0,
            edges_added: g.edges.len() as u64,
            edges_removed: 0,
            compacted_from: None,
            replaced_chain: Vec::new(),
        }
    }

    /// Counters implied by a delta payload.
    pub fn from_delta(d: &GraphDelta) -> Self {
        Self {
            files: d.files.len() as u64,
            nodes: d.nodes_added.len() as u64,
            edges: d.edges_added.len() as u64,
            unresolved_refs: d
                .unresolved_replaced
                .iter()
                .map(|(_, rows)| rows.len() as u64)
                .sum(),
            parse_errors: 0,
            edges_added: d.edges_added.len() as u64,
            edges_removed: d.edges_removed.len() as u64,
            compacted_from: None,
            replaced_chain: Vec::new(),
        }
    }
}

/// Everything that identifies a new snapshot row. Status starts at
/// [`SnapshotStatus::Pending`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSnapshot {
    pub scope: RepoScope,
    pub commit_sha: CommitSha,
    pub kind: SnapshotKind,
    /// Required for [`SnapshotKind::Delta`], forbidden for [`SnapshotKind::Full`].
    pub base: Option<SnapshotId>,
    pub purpose: SnapshotPurpose,
    pub versions: SnapshotVersions,
}

impl NewSnapshot {
    /// A full snapshot of `scope` at `commit_sha`.
    pub fn full(
        scope: RepoScope,
        commit_sha: CommitSha,
        purpose: SnapshotPurpose,
        fingerprint: [u8; 32],
    ) -> Self {
        Self {
            scope,
            commit_sha,
            kind: SnapshotKind::Full,
            base: None,
            purpose,
            versions: SnapshotVersions::new(fingerprint),
        }
    }

    /// A delta on top of `base`.
    pub fn delta(
        scope: RepoScope,
        commit_sha: CommitSha,
        purpose: SnapshotPurpose,
        base: SnapshotId,
        fingerprint: [u8; 32],
    ) -> Self {
        Self {
            scope,
            commit_sha,
            kind: SnapshotKind::Delta,
            base: Some(base),
            purpose,
            versions: SnapshotVersions::new(fingerprint),
        }
    }

    /// Checks the invariants the schema enforces with CHECK constraints.
    pub fn validate(&self) -> Result<(), crate::StoreError> {
        match (self.kind, self.base) {
            (SnapshotKind::Full, None) => Ok(()),
            (SnapshotKind::Delta, Some(_)) => Ok(()),
            (SnapshotKind::Full, Some(_)) => Err(crate::StoreError::InvalidRequest(
                "a full snapshot must not name a base".to_owned(),
            )),
            (SnapshotKind::Delta, None) => Err(crate::StoreError::InvalidRequest(
                "a delta snapshot must name its base".to_owned(),
            )),
        }
    }
}

/// The stored form of a snapshot, as returned by `GraphStore::snapshot` and friends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    pub id: SnapshotId,
    pub scope: RepoScope,
    pub commit_sha: CommitSha,
    pub kind: SnapshotKind,
    pub base: Option<SnapshotId>,
    /// Distance from the base full snapshot: 0 for `full`, `base + 1` for a delta.
    pub chain_depth: u16,
    pub purpose: SnapshotPurpose,
    pub status: SnapshotStatus,
    pub versions: SnapshotVersions,
    pub stats: SnapshotStats,
    /// Failure reason; set when the status is [`SnapshotStatus::Failed`].
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl SnapshotMeta {
    /// True when this snapshot starts a chain (it is a full snapshot).
    pub fn is_chain_root(&self) -> bool {
        self.base.is_none()
    }
}

impl JsonSchema for SnapshotMeta {
    fn schema_name() -> String {
        "SnapshotMeta".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        // `RepoScope` lives in `repository` and does not implement `JsonSchema`, so the mirror
        // struct carries its two ids instead and the generator produces the object schema.
        #[derive(JsonSchema)]
        #[allow(dead_code)]
        struct Mirror {
            id: SnapshotId,
            organization_id: OrganizationId,
            repository_id: RepositoryId,
            commit_sha: CommitSha,
            kind: SnapshotKind,
            base: Option<SnapshotId>,
            chain_depth: u16,
            purpose: SnapshotPurpose,
            status: SnapshotStatus,
            versions: SnapshotVersions,
            stats: SnapshotStats,
            error: Option<String>,
            created_at: DateTime<Utc>,
            updated_at: DateTime<Utc>,
            completed_at: Option<DateTime<Utc>>,
        }
        let mut schema = Mirror::json_schema(generator);
        if let Schema::Object(SchemaObject {
            metadata: Some(metadata),
            ..
        }) = &mut schema
        {
            metadata.title = Some("SnapshotMeta".to_owned());
        }
        schema
    }
}

/// Selector for `GraphStore::find_ready`. `None` fields do not constrain the search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotQuery {
    pub commit_sha: Option<CommitSha>,
    pub fingerprint: Option<[u8; 32]>,
    pub kind: Option<SnapshotKind>,
    pub purpose: Option<SnapshotPurpose>,
}

impl SnapshotQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn by_commit(commit_sha: CommitSha) -> Self {
        Self {
            commit_sha: Some(commit_sha),
            ..Self::default()
        }
    }

    pub fn by_fingerprint(fingerprint: [u8; 32]) -> Self {
        Self {
            fingerprint: Some(fingerprint),
            ..Self::default()
        }
    }

    pub fn with_kind(mut self, kind: SnapshotKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn with_purpose(mut self, purpose: SnapshotPurpose) -> Self {
        self.purpose = Some(purpose);
        self
    }
}

/// One content-addressed file version handed to `GraphStore::upsert_file_versions`.
#[derive(Debug, Clone, PartialEq)]
pub struct FileVersionInput {
    pub path: String,
    /// Blake3-256 of the file bytes.
    pub content_hash: [u8; 32],
    pub language: String,
    pub analyzer_version: String,
    pub parse_status: FileParseStatus,
    pub size_bytes: u32,
    pub diagnostic_count: u32,
    /// The complete symbol set of this file version (the invariant GS-004 tests).
    pub symbols: Vec<Node>,
}

impl FileVersionInput {
    pub fn key(&self) -> FileVersionKey {
        FileVersionKey {
            path: self.path.clone(),
            content_hash: self.content_hash,
            analyzer_version: self.analyzer_version.clone(),
        }
    }
}

/// The content-addressed identity of a file version (ADR-003).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileVersionKey {
    pub path: String,
    pub content_hash: [u8; 32],
    pub analyzer_version: String,
}

impl FileVersionKey {
    /// `<path> NUL <hash hex> NUL <analyzer>` — the file adapter's manifest key (GS-006).
    pub fn manifest_key(&self) -> String {
        let mut out = String::with_capacity(self.path.len() + 64 + self.analyzer_version.len() + 2);
        out.push_str(&self.path);
        out.push('\0');
        out.push_str(&hex_lower(&self.content_hash));
        out.push('\0');
        out.push_str(&self.analyzer_version);
        out
    }
}

/// The adapter-allocated id of a stored file version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersionRef {
    pub id: i64,
    pub path: String,
    pub content_hash: [u8; 32],
    pub analyzer_version: String,
}

/// Lowercase hex without pulling in a `hex` dependency.
pub fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 0x0f)] as char);
    }
    out
}

/// Parses lowercase hex written by [`hex_lower`].
pub fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in bytes.chunks(2) {
        out.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
    }
    Some(out)
}

/// Rows touched by one `write_full`/`write_delta` call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteStats {
    pub files: u64,
    pub nodes: u64,
    pub edges: u64,
    pub unresolved: u64,
    pub lineage: u64,
    /// File versions the write needed that already existed.
    pub file_versions_existing: u64,
    pub bytes: u64,
}

impl WriteStats {
    pub fn total_rows(&self) -> u64 {
        self.files + self.nodes + self.edges + self.unresolved + self.lineage
    }
}

/// One page of `GraphStore::neighbors` results.
#[derive(Debug, Clone, PartialEq)]
pub struct NeighborPage {
    /// Stored edges (never tombstones), ordered by `(kind, other_key)` — the same order the
    /// in-memory `Graph::neighbors` produces.
    pub edges: Vec<crate::model::GraphEdge>,
    /// Continue after this point when the page was filled; `None` at the end.
    pub next_cursor: Option<crate::model::EdgeCursor>,
}

impl NeighborPage {
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

/// `crate::model::Node`, named for the port.
pub type StoredNode = Node;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::kinds::{
        Confidence, EdgeFlags, EdgeKind, NodeKey, NodeKind, Provenance, ResolvedBy,
    };
    use crate::model::{EdgeCursor, FileChange, GraphEdge, SnapshotFile};

    fn sha(c: char) -> CommitSha {
        c.to_string().repeat(40).parse().unwrap()
    }

    fn scope() -> RepoScope {
        RepoScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        }
    }

    #[test]
    fn new_snapshot_validates_kind_against_base() {
        let mut full = NewSnapshot::full(scope(), sha('a'), SnapshotPurpose::Local, [1u8; 32]);
        full.validate().unwrap();
        full.base = Some(SnapshotId::new());
        assert!(matches!(
            full.validate(),
            Err(crate::StoreError::InvalidRequest(_))
        ));

        let mut delta = NewSnapshot::delta(
            scope(),
            sha('b'),
            SnapshotPurpose::PullRequest,
            SnapshotId::new(),
            [2u8; 32],
        );
        delta.validate().unwrap();
        delta.base = None;
        assert!(delta.validate().is_err());
    }

    #[test]
    fn kind_and_purpose_roundtrip_through_db_text() {
        for kind in [SnapshotKind::Full, SnapshotKind::Delta] {
            assert_eq!(SnapshotKind::from_db(kind.as_str()), Some(kind));
        }
        for purpose in [
            SnapshotPurpose::DefaultBranch,
            SnapshotPurpose::PullRequest,
            SnapshotPurpose::Local,
            SnapshotPurpose::Compaction,
        ] {
            assert_eq!(SnapshotPurpose::from_db(purpose.as_str()), Some(purpose));
        }
        assert_eq!(SnapshotKind::from_db("delta "), None);
        assert_eq!(SnapshotPurpose::from_db("head"), None);
        for status in [
            FileParseStatus::Ok,
            FileParseStatus::Partial,
            FileParseStatus::Failed,
            FileParseStatus::Skipped,
        ] {
            assert_eq!(FileParseStatus::from_db(status.as_str()), Some(status));
        }
    }

    #[test]
    fn stats_from_graph_and_delta_count_the_expected_rows() {
        let g = Graph {
            schema_version: 1,
            files: vec![SnapshotFile {
                path: "src/a.ts".to_owned(),
                file_version_id: Some(1),
                change: FileChange::Present,
                old_path: None,
            }],
            nodes: Vec::new(),
            edges: Vec::new(),
            unresolved: Vec::new(),
        };
        let stats = SnapshotStats::from_graph(&g);
        assert_eq!(stats.files, 1);
        assert_eq!(stats.edges_added, 0);

        let delta = GraphDelta {
            edges_added: vec![GraphEdge {
                source: NodeKey::from_bytes([1; 16]),
                kind: EdgeKind::Calls,
                target: NodeKey::from_bytes([2; 16]),
                confidence: Confidence::from_permille(900),
                resolved_by: ResolvedBy::NameUnique,
                provenance: Provenance::Linker,
                flags: EdgeFlags::EMPTY,
                origin_path: None,
                line: None,
                col: None,
                occurrences: 1,
            }],
            ..GraphDelta::default()
        };
        let stats = SnapshotStats::from_delta(&delta);
        assert_eq!(stats.edges_added, 1);
        assert_eq!(stats.edges_removed, 0);
        assert_eq!(stats.edges, 1);
        assert_eq!(stats.unresolved_refs, 0);
    }

    #[test]
    fn snapshot_meta_has_a_json_schema() {
        let schema = schemars::schema_for!(SnapshotMeta);
        let text = serde_json::to_value(&schema).unwrap().to_string();
        assert!(text.contains("SnapshotMeta"), "{text}");
        assert!(text.contains("organization_id"), "{text}");
        assert!(text.contains("chain_depth"), "{text}");
    }

    #[test]
    fn versions_default_to_the_current_schema_version() {
        let versions = SnapshotVersions::default();
        assert_eq!(versions.graph_schema_version, GRAPH_SCHEMA_VERSION);
        assert!(versions.analyzer_versions.is_empty());
        assert_eq!(versions.fingerprint, [0u8; 32]);
    }

    #[test]
    fn file_version_manifest_key_is_path_nul_hash_nul_analyzer() {
        let key = FileVersionKey {
            path: "src/a.ts".to_owned(),
            content_hash: [0xabu8; 32],
            analyzer_version: "1.0.0".to_owned(),
        };
        let shown = key.manifest_key();
        assert!(shown.starts_with("src/a.ts\u{0}"));
        assert!(shown.ends_with("\u{0}1.0.0"));
        assert_eq!(
            shown.split('\0').nth(1),
            Some("ab".repeat(32).as_str()),
            "hash is lowercase hex"
        );
        assert_eq!(
            hex_bytes(shown.split('\0').nth(1).unwrap()).unwrap(),
            vec![0xabu8; 32]
        );
        assert!(hex_bytes("abc").is_none());
        assert!(hex_bytes("zz").is_none());
    }

    #[test]
    fn write_stats_sum_and_neighbor_page_helpers() {
        let stats = WriteStats {
            files: 1,
            nodes: 2,
            edges: 3,
            unresolved: 4,
            lineage: 5,
            file_versions_existing: 6,
            bytes: 7,
        };
        assert_eq!(stats.total_rows(), 15);
        let page = NeighborPage {
            edges: Vec::new(),
            next_cursor: None,
        };
        assert!(page.is_empty());
        assert_eq!(
            EdgeCursor {
                kind: EdgeKind::Calls,
                key: NodeKey::from_bytes([9; 16])
            }
            .key,
            NodeKey::from_bytes([9; 16])
        );
        let _ = NodeKind::Function;
    }
}
