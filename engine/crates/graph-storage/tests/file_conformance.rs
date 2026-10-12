//! GS-006: the file adapter against the GS-001 conformance suite, plus its crash-safety and
//! integrity cases. Runs with `cargo test -p graph-storage --features conformance`.

#![cfg(feature = "conformance")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use graph_storage::conformance::{
    file_version_inputs, fixture_graph, run_all, sha, with_file_versions, Harness, HarnessFixture,
    FINGERPRINT,
};
use graph_storage::file::atomic::write_atomic;
use graph_storage::file::FileGraphStore;
use graph_storage::status::SnapshotStatus;
use graph_storage::types::{NewSnapshot, SnapshotPurpose};
use graph_storage::{GraphStore, StoreError};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};
use tempfile::TempDir;

#[derive(Debug, Default)]
struct FileHarness {
    dirs: Mutex<Vec<TempDir>>,
}

fn scope() -> RepoScope {
    RepoScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

#[async_trait]
impl Harness for FileHarness {
    fn name(&self) -> &str {
        "file"
    }

    async fn fresh(&self) -> Result<HarnessFixture, StoreError> {
        let dir = tempfile::tempdir().map_err(StoreError::backend)?;
        let store = FileGraphStore::open(dir.path().join(".review/graph"))?;
        self.dirs
            .lock()
            .map_err(|_| StoreError::backend_msg("harness lock poisoned"))?
            .push(dir);
        Ok(HarnessFixture {
            store: Arc::new(store),
            scope: scope(),
            foreign: scope(),
        })
    }
}

async fn ready_full(f: &HarnessFixture) -> SnapshotId {
    let g = with_file_versions(f, &fixture_graph()).await.unwrap();
    let meta = f
        .store
        .create_snapshot(NewSnapshot::full(
            f.scope,
            sha('a').unwrap(),
            SnapshotPurpose::Local,
            FINGERPRINT,
        ))
        .await
        .unwrap();
    for (from, to) in [
        (SnapshotStatus::Pending, SnapshotStatus::Indexing),
        (SnapshotStatus::Indexing, SnapshotStatus::Persisting),
    ] {
        assert!(f.store.transition(&f.scope, meta.id, from, to, None).await.unwrap());
    }
    f.store.write_full(&f.scope, meta.id, &g).await.unwrap();
    assert!(f
        .store
        .transition(
            &f.scope,
            meta.id,
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready,
            None
        )
        .await
        .unwrap());
    meta.id
}

fn fixture_at(root: &std::path::Path, scope: RepoScope) -> HarnessFixture {
    HarnessFixture {
        store: Arc::new(FileGraphStore::open(root).unwrap()),
        scope,
        foreign: RepoScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        },
    }
}

#[tokio::test]
async fn file_conformance_suite() {
    let report = run_all(&FileHarness::default()).await;
    assert_eq!(report.adapter, "file");
    report.assert_ok();
}

#[tokio::test]
async fn reopened_store_reads_what_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let scope = scope();
    let id = ready_full(&fixture_at(&root, scope)).await;
    let reopened = fixture_at(&root, scope);
    let loaded = reopened.store.load_graph(&scope, id).await.unwrap();
    assert_eq!(loaded.edges.len(), fixture_graph().edges.len());
}

#[tokio::test]
async fn crash_before_manifest_commit_keeps_old_state() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let scope = scope();
    let id = ready_full(&fixture_at(&root, scope)).await;
    let manifest_before = std::fs::read(root.join("manifest.json")).unwrap();
    // A payload file reached disk but the process died before the manifest named it.
    write_atomic(
        &root.join("snapshots/00000000000000000000000000000000.bin.zst"),
        b"orphan",
    )
    .unwrap();
    let reopened = fixture_at(&root, scope);
    assert_eq!(std::fs::read(root.join("manifest.json")).unwrap(), manifest_before);
    assert!(reopened.store.load_graph(&scope, id).await.is_ok());
}

#[tokio::test]
async fn tmp_files_cleaned_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    drop(FileGraphStore::open(&root).unwrap());
    std::fs::write(root.join("manifest.json.tmp-123-abc"), b"partial").unwrap();
    std::fs::write(root.join("snapshots/x.bin.zst.tmp-9-z"), b"partial").unwrap();
    drop(FileGraphStore::open(&root).unwrap());
    assert!(!root.join("manifest.json.tmp-123-abc").exists());
    assert!(!root.join("snapshots/x.bin.zst.tmp-9-z").exists());
}

#[tokio::test]
async fn hash_mismatch_marks_inconsistent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let scope = scope();
    let f = fixture_at(&root, scope);
    let id = ready_full(&f).await;
    let file = root.join(format!("snapshots/{}.bin.zst", id.as_uuid().simple()));
    let mut bytes = std::fs::read(&file).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&file, bytes).unwrap();
    match f.store.load_graph(&scope, id).await {
        Err(StoreError::Integrity(_)) => {}
        other => panic!("expected an integrity error, got {other:?}"),
    }
    let meta = f.store.snapshot(&scope, id).await.unwrap().unwrap();
    assert_eq!(meta.status, SnapshotStatus::Inconsistent);
}

#[tokio::test]
async fn corrupt_manifest_is_integrity_error() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let f = fixture_at(&root, scope());
    std::fs::write(root.join("manifest.json"), b"{ not json").unwrap();
    match f.store.snapshot(&f.scope, SnapshotId::new()).await {
        Err(StoreError::Integrity(message)) => assert!(message.contains("rebuild")),
        other => panic!("expected an integrity error, got {other:?}"),
    }
}

#[tokio::test]
async fn rename_replaces_existing() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("file.json");
    write_atomic(&target, b"first").unwrap();
    write_atomic(&target, b"second").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"second");
}

#[tokio::test]
async fn two_stores_on_one_directory_serialize_writes() {
    // Two handles on the same directory stand in for two CLI processes: the advisory lock
    // serializes them, so concurrent upserts converge on one id per key.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    let scope = scope();
    let first = fixture_at(&root, scope);
    let second = fixture_at(&root, scope);
    let inputs = file_version_inputs(&fixture_graph());
    let (a, b) = tokio::join!(
        first.store.upsert_file_versions(&scope, &inputs),
        second.store.upsert_file_versions(&scope, &inputs)
    );
    let a: Vec<i64> = a.unwrap().iter().map(|r| r.id).collect();
    let b: Vec<i64> = b.unwrap().iter().map(|r| r.id).collect();
    assert_eq!(a, b);
}
