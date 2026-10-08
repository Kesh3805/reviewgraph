//! The shared conformance suite (GS-001).
//!
//! One suite, three adapters: [`MemGraphStore`](crate::mem), the Postgres adapter
//! (`tests/pg_conformance.rs`) and the file adapter (`tests/file_conformance.rs`) all call
//! [`run_all`] and must report zero failures. Fixtures come from this module, so an adapter
//! cannot pass by choosing its own inputs.
//!
//! Cases return `Result<(), String>` instead of asserting, which lets [`run_all`] collect
//! *every* failure rather than stopping at the first one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use repository::store::RepoScope;
use review_core::ids::{CommitSha, SnapshotId};

use crate::kinds::{
    Confidence, Direction, EdgeFlags, EdgeKind, EdgeKindSet, NodeKey, NodeKind, Provenance,
    ResolvedBy,
};
use crate::model::{
    FileChange, Graph, GraphDelta, GraphEdge, Node, SnapshotFile, SourceRange, SymbolNode,
    SyntheticNode, UnresolvedRef,
};
use crate::port::GraphStore;
use crate::status::SnapshotStatus;
use crate::types::{
    FileParseStatus, FileVersionInput, NewSnapshot, SnapshotMeta, SnapshotPurpose,
    GRAPH_SCHEMA_VERSION,
};
use crate::StoreError;

pub mod concurrency;
pub mod delta;
pub mod neighbors;
pub mod roundtrip;
pub mod status;

/// Fingerprint used by every fixture that does not care about it.
pub const FINGERPRINT: [u8; 32] = [7u8; 32];
/// A second fingerprint, so `find_ready` can tell two snapshots apart.
pub const OTHER_FINGERPRINT: [u8; 32] = [9u8; 32];

/// Everything one case runs against: a store plus the two scopes it must isolate.
#[derive(Clone)]
pub struct HarnessFixture {
    pub store: Arc<dyn GraphStore>,
    /// The tenant the fixture data belongs to.
    pub scope: RepoScope,
    /// A different tenant in the same store, for cross-tenant cases.
    pub foreign: RepoScope,
}

impl fmt::Debug for HarnessFixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HarnessFixture")
            .field("scope", &self.scope)
            .field("foreign", &self.foreign)
            .finish()
    }
}

/// Supplies one isolated fixture per case. `name` labels the report.
#[async_trait]
pub trait Harness: Send + Sync {
    fn name(&self) -> &str;
    /// A store with `scope` (and a distinct `foreign` scope) ready to use.
    async fn fresh(&self) -> Result<HarnessFixture, StoreError>;
}

/// One failed case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseFailure {
    pub case: &'static str,
    pub message: String,
}

/// The outcome of a whole run.
#[derive(Debug, Clone, Default)]
pub struct ConformanceReport {
    pub adapter: String,
    /// How many cases ran; must equal [`CASE_COUNT`].
    pub ran: usize,
    pub failures: Vec<CaseFailure>,
}

impl ConformanceReport {
    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }

    /// Panics with every failure listed; call from a test after [`run_all`].
    pub fn assert_ok(&self) {
        assert_eq!(
            self.ran, CASE_COUNT,
            "the suite ran {} of {CASE_COUNT} cases",
            self.ran
        );
        assert!(
            self.is_ok(),
            "{} conformance suite failed ({} case(s)): {}",
            self.adapter,
            self.failures.len(),
            self.failures
                .iter()
                .map(|f| format!("\n  {} — {}", f.case, f.message))
                .collect::<String>()
        );
    }
}

/// Runs every case in isolation and reports all failures, not just the first.
pub async fn run_all(h: &dyn Harness) -> ConformanceReport {
    let mut report = ConformanceReport {
        adapter: h.name().to_owned(),
        ran: 0,
        failures: Vec::new(),
    };
    macro_rules! run {
        ($case:path) => {
            report.ran += 1;
            match $case(h).await {
                Ok(()) => {}
                Err(message) => report.failures.push(CaseFailure {
                    case: stringify!($case),
                    message,
                }),
            }
        };
    }
    run!(roundtrip::write_full_then_load_roundtrip);
    run!(roundtrip::find_ready_by_commit_and_fingerprint);
    run!(roundtrip::cross_repository_snapshot_is_not_found);
    run!(roundtrip::nodes_lookup_matches_graph);
    run!(delta::write_delta_then_load_equals_overlay_flatten);
    run!(delta::three_level_delta_chain_materializes);
    run!(delta::tombstone_removes_edge);
    run!(delta::edge_override_in_delta);
    run!(delta::deleted_file_removes_nodes_and_unresolved);
    run!(delta::relinked_file_replaces_unresolved_rows);
    run!(delta::delta_on_non_ready_base_rejected);
    run!(status::status_cas_only_one_winner);
    run!(status::illegal_transition_returns_false);
    run!(status::load_non_ready_snapshot_errors);
    run!(status::write_to_ready_snapshot_errors);
    run!(neighbors::neighbors_match_in_memory_query);
    run!(concurrency::upsert_file_versions_is_idempotent_under_concurrency);
    report
}

/// Number of cases in the suite; a drift guard for the report.
pub const CASE_COUNT: usize = 17;

// ---------------------------------------------------------------- fixtures

/// A 40-character commit sha made of one repeated character.
pub fn sha(c: char) -> Result<CommitSha, String> {
    c.to_string()
        .repeat(40)
        .parse()
        .map_err(|e| format!("invalid commit sha: {e}"))
}

/// Shows any store error without needing `unwrap`.
pub fn show(e: StoreError) -> String {
    e.to_string()
}

/// A node key from a single byte, so fixtures stay readable.
pub fn key(byte: u8) -> NodeKey {
    NodeKey::from_bytes([byte; 16])
}

/// A symbol node owned by `file`.
pub fn symbol(byte: u8, file: &str, name: &str) -> Node {
    Node::Symbol(SymbolNode {
        key: key(byte),
        id: format!("ts:{file}#{name}"),
        kind: NodeKind::Function,
        name: name.to_owned(),
        qualified_name: name.to_owned(),
        file: file.to_owned(),
        range: SourceRange {
            start_line: 1,
            start_col: 1,
            end_line: 2,
            end_col: 2,
        },
        signature: None,
        body_hash: None,
        signature_hash: None,
        parent: None,
        visibility: 0,
        is_exported: false,
        is_generated: false,
        attrs: serde_json::json!({}),
    })
}

/// An edge from `from` to `to`.
pub fn edge(from: u8, kind: EdgeKind, to: u8, permille: u16) -> GraphEdge {
    GraphEdge {
        source: key(from),
        kind,
        target: key(to),
        confidence: Confidence::from_permille(permille),
        resolved_by: ResolvedBy::NameUnique,
        provenance: Provenance::Linker,
        flags: EdgeFlags::EMPTY,
        origin_path: Some("src/a.ts".to_owned()),
        line: Some(1),
        col: Some(1),
        occurrences: 1,
    }
}

/// A file list entry.
pub fn file(path: &str, change: FileChange) -> SnapshotFile {
    SnapshotFile {
        path: path.to_owned(),
        file_version_id: None,
        change,
        old_path: None,
    }
}

/// An unresolved reference row of `path`.
pub fn unresolved(path: &str, ordinal: u32, name: &str) -> UnresolvedRef {
    UnresolvedRef {
        file: path.to_owned(),
        ordinal,
        from: None,
        name: name.to_owned(),
        ref_kind: 0,
        import_specifier: None,
        reason: 0,
        candidate_count: 0,
        line: 1,
        col: 1,
    }
}

/// The deterministic full-graph fixture: three files, five nodes, six edges, one unresolved
/// reference. Normalized, so `write_full`/`load_graph` round-trips it byte for byte.
pub fn fixture_graph() -> Graph {
    Graph {
        schema_version: GRAPH_SCHEMA_VERSION,
        files: vec![
            file("src/a.ts", FileChange::Present),
            file("src/b.ts", FileChange::Present),
            file("src/c.ts", FileChange::Present),
        ],
        nodes: vec![
            symbol(1, "src/a.ts", "alpha"),
            symbol(2, "src/a.ts", "alpha2"),
            symbol(3, "src/b.ts", "beta"),
            symbol(4, "src/c.ts", "gamma"),
            Node::Synthetic(SyntheticNode {
                key: key(5),
                id: "queue:emails".to_owned(),
                kind: NodeKind::Queue,
                attrs: serde_json::json!({}),
            }),
        ],
        edges: vec![
            edge(1, EdgeKind::Calls, 3, 900),
            edge(1, EdgeKind::Calls, 4, 400),
            edge(1, EdgeKind::Imports, 3, 1000),
            edge(2, EdgeKind::References, 3, 950),
            edge(3, EdgeKind::Calls, 1, 900),
            edge(5, EdgeKind::HandledBy, 3, 700),
        ],
        unresolved: vec![unresolved("src/b.ts", 0, "missing")],
    }
    .normalized()
}

/// Stable pseudo content hash of a path: file versions are content addressed, and fixtures
/// must not need a hashing dependency to agree on one.
pub fn content_hash(path: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in path.bytes().enumerate() {
        out[i % 32] ^= b;
        out[i % 32] = out[i % 32].wrapping_add(i as u8);
    }
    out
}

/// The file-version inputs implied by a graph's present files.
pub fn file_version_inputs(g: &Graph) -> Vec<FileVersionInput> {
    g.files
        .iter()
        .filter(|f| f.file_version_id.is_none() && f.change != FileChange::Deleted)
        .map(|f| FileVersionInput {
            path: f.path.clone(),
            content_hash: content_hash(&f.path),
            language: "typescript".to_owned(),
            analyzer_version: "0.3.1".to_owned(),
            parse_status: FileParseStatus::Ok,
            size_bytes: 128,
            diagnostic_count: 0,
            symbols: g
                .nodes
                .iter()
                .filter(|n| n.file() == Some(f.path.as_str()))
                .cloned()
                .collect(),
        })
        .collect()
}

/// Upserts the graph's file versions and fills `file_version_id` on every listed path, so a
/// snapshot written by an adapter with a `file_versions` FK is complete.
pub async fn with_file_versions(f: &HarnessFixture, g: &Graph) -> Result<Graph, String> {
    let inputs = file_version_inputs(g);
    if inputs.is_empty() {
        return Ok(g.clone());
    }
    let refs = f
        .store
        .upsert_file_versions(&f.scope, &inputs)
        .await
        .map_err(show)?;
    let mut by_path = std::collections::BTreeMap::new();
    for r in refs {
        by_path.insert(r.path, r.id);
    }
    let mut out = g.clone();
    for file in &mut out.files {
        if let Some(id) = by_path.get(&file.path) {
            file.file_version_id = Some(*id);
        }
    }
    Ok(out.normalized())
}

// ---------------------------------------------------------------- helpers

/// Fails the case when the transition was refused.
pub async fn must_transition(
    f: &HarnessFixture,
    id: SnapshotId,
    from: SnapshotStatus,
    to: SnapshotStatus,
) -> Result<SnapshotMeta, String> {
    let moved = f
        .store
        .transition(&f.scope, id, from, to, None)
        .await
        .map_err(show)?;
    if !moved {
        return Err(format!("transition {from} -> {to} was refused"));
    }
    fetch(f, id).await
}

/// Current metadata of `id`.
pub async fn fetch(f: &HarnessFixture, id: SnapshotId) -> Result<SnapshotMeta, String> {
    f.store
        .snapshot(&f.scope, id)
        .await
        .map_err(show)?
        .ok_or_else(|| format!("snapshot {id} vanished"))
}

/// `Pending -> Indexing -> Persisting`, so the snapshot accepts a payload.
pub async fn to_persisting(f: &HarnessFixture, id: SnapshotId) -> Result<SnapshotMeta, String> {
    must_transition(f, id, SnapshotStatus::Pending, SnapshotStatus::Indexing).await?;
    must_transition(f, id, SnapshotStatus::Indexing, SnapshotStatus::Persisting).await
}

/// Creates a full snapshot in `Persisting`, ready to be written.
pub async fn pending_full(
    f: &HarnessFixture,
    commit: CommitSha,
    purpose: SnapshotPurpose,
    fingerprint: [u8; 32],
) -> Result<SnapshotMeta, String> {
    let meta = f
        .store
        .create_snapshot(NewSnapshot::full(f.scope, commit, purpose, fingerprint))
        .await
        .map_err(show)?;
    to_persisting(f, meta.id).await
}

/// Creates a delta snapshot in `Persisting` on top of `base`.
pub async fn pending_delta(
    f: &HarnessFixture,
    commit: CommitSha,
    purpose: SnapshotPurpose,
    fingerprint: [u8; 32],
    base: SnapshotId,
) -> Result<SnapshotMeta, String> {
    let meta = f
        .store
        .create_snapshot(NewSnapshot::delta(
            f.scope,
            commit,
            purpose,
            base,
            fingerprint,
        ))
        .await
        .map_err(show)?;
    to_persisting(f, meta.id).await
}

/// A full snapshot written and transitioned to `Ready`.
pub async fn ready_full(
    f: &HarnessFixture,
    g: &Graph,
    commit: CommitSha,
    fingerprint: [u8; 32],
) -> Result<SnapshotMeta, String> {
    let g = with_file_versions(f, g).await?;
    let meta = pending_full(f, commit, SnapshotPurpose::DefaultBranch, fingerprint).await?;
    f.store
        .write_full(&f.scope, meta.id, &g)
        .await
        .map_err(show)?;
    must_transition(
        f,
        meta.id,
        SnapshotStatus::Persisting,
        SnapshotStatus::Ready,
    )
    .await
}

/// A delta written and transitioned to `Ready`.
pub async fn ready_delta(
    f: &HarnessFixture,
    d: &GraphDelta,
    commit: CommitSha,
    fingerprint: [u8; 32],
    base: SnapshotId,
) -> Result<SnapshotMeta, String> {
    let meta = pending_delta(f, commit, SnapshotPurpose::DefaultBranch, fingerprint, base).await?;
    f.store
        .write_delta(&f.scope, meta.id, d)
        .await
        .map_err(show)?;
    must_transition(
        f,
        meta.id,
        SnapshotStatus::Persisting,
        SnapshotStatus::Ready,
    )
    .await
}

/// Every edge kind, for "no filter" neighbour queries.
pub fn all_kinds() -> EdgeKindSet {
    EdgeKindSet::ALL
}

/// Out direction, default confidence, wide limit — the common neighbour query.
pub fn wide_query() -> (Direction, EdgeKindSet, Confidence, u32) {
    (Direction::Out, EdgeKindSet::ALL, Confidence::MIN, 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_are_normalized_and_distinct() {
        let g = fixture_graph();
        assert_eq!(g.nodes.len(), 5);
        assert_eq!(g.edges.len(), 6);
        assert_eq!(g.files.len(), 3);
        assert_eq!(g.unresolved.len(), 1);
        assert_eq!(g, g.clone().normalized());
        assert_ne!(content_hash("src/a.ts"), content_hash("src/b.ts"));
        assert_eq!(content_hash("src/a.ts"), content_hash("src/a.ts"));
    }

    #[test]
    fn case_count_matches_the_suite() {
        assert_eq!(CASE_COUNT, 17);
    }

    #[tokio::test]
    async fn sha_is_forty_repeated_characters() {
        let sha = sha('a').expect("valid fixture sha");
        assert_eq!(sha.to_string(), "a".repeat(40));
        assert!(sha.to_string().parse::<CommitSha>().is_ok());
    }
}
