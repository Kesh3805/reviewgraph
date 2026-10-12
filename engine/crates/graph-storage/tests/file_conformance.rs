//! GS-006: the file adapter against the shared conformance suite, plus its own failure cases
//! (crash before the manifest commit, leftover temp files, hash mismatch, two processes,
//! retention).
//!
//! Runs with `cargo test -p graph-storage --features conformance`.

#![cfg(feature = "conformance")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use graph_storage::conformance::{
    fixture_graph, must_transition, pending_delta, ready_delta, ready_full, run_all, sha, Harness,
    HarnessFixture, FINGERPRINT, OTHER_FINGERPRINT,
};
use graph_storage::file::{FileGraphStore, FileStoreOptions};
use graph_storage::model::GraphDelta;
use graph_storage::status::SnapshotStatus;
use graph_storage::types::{NewSnapshot, SnapshotPurpose};
use graph_storage::{GraphStore, StoreError};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId};

/// Environment variable that turns [`lock_holder_child`] into a lock-holding helper process.
const CHILD_ENV: &str = "RG_GS006_LOCK_FILE";

#[derive(Debug, Default)]
struct FileHarness {
    dirs: Mutex<Vec<tempfile::TempDir>>,
}

#[async_trait]
impl Harness for FileHarness {
    fn name(&self) -> &str {
        "file"
    }

    async fn fresh(&self) -> Result<HarnessFixture, StoreError> {
        let dir = tempfile::tempdir().map_err(StoreError::backend)?;
        let store = FileGraphStore::open(dir.path().join("graph")).await?;
        self.dirs.lock().unwrap().push(dir);
        Ok(HarnessFixture {
            store: Arc::new(store),
            scope: scope(),
            foreign: scope(),
        })
    }
}

fn scope() -> RepoScope {
    RepoScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

async fn fresh_store(retention: usize) -> (tempfile::TempDir, FileGraphStore, RepoScope) {
    let dir = tempfile::tempdir().unwrap();
    let store = FileGraphStore::open_with(
        dir.path().join("graph"),
        FileStoreOptions {
            retention,
            ..FileStoreOptions::default()
        },
    )
    .await
    .unwrap();
    (dir, store, scope())
}

fn snapshot_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("snapshots"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
}

fn fixture_of(store: FileGraphStore, scope: RepoScope) -> HarnessFixture {
    HarnessFixture {
        store: Arc::new(store),
        scope,
        foreign: self::scope(),
    }
}

#[tokio::test]
async fn file_conformance_suite() {
    let harness = FileHarness::default();
    let report = run_all(&harness).await;
    assert_eq!(report.adapter, "file");
    report.assert_ok();
}

#[tokio::test]
async fn crash_before_manifest_commit_keeps_old_state() {
    let (dir, store, scope) = fresh_store(10).await;
    let root = store.root().to_path_buf();
    let meta = store
        .create_snapshot(NewSnapshot::full(
            scope,
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
        assert!(store
            .transition(&scope, meta.id, from, to, None)
            .await
            .unwrap());
    }

    // The snapshot file becomes durable, then the manifest commit fails.
    store.fail_next_manifest_commit();
    let graph = fixture_graph();
    assert!(store.write_full(&scope, meta.id, &graph).await.is_err());
    assert_eq!(
        snapshot_files(&root).len(),
        1,
        "the orphaned snapshot file is on disk"
    );

    // A new handle sees the old manifest: still Persisting, nothing written.
    drop(store);
    let reopened = FileGraphStore::open(&root).await.unwrap();
    let seen = reopened.snapshot(&scope, meta.id).await.unwrap().unwrap();
    assert_eq!(seen.status, SnapshotStatus::Persisting);
    assert_eq!(seen.stats.nodes, 0, "the failed write left no counters");

    // The retry overwrites the orphan and the snapshot becomes readable.
    reopened.write_full(&scope, meta.id, &graph).await.unwrap();
    assert_eq!(snapshot_files(&root).len(), 1, "the retry reuses the name");
    assert!(reopened
        .transition(
            &scope,
            meta.id,
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready,
            None
        )
        .await
        .unwrap());
    let loaded = reopened.load_graph(&scope, meta.id).await.unwrap();
    assert_eq!(loaded, graph.normalized());
    drop(dir);
}

#[tokio::test]
async fn tmp_files_cleaned_on_open() {
    let (dir, store, _scope) = fresh_store(10).await;
    let root = store.root().to_path_buf();
    drop(store);
    std::fs::write(root.join("manifest.json.tmp-1-abc"), b"partial").unwrap();
    std::fs::write(
        root.join("snapshots").join("x.bin.zst.tmp-9-def"),
        b"partial",
    )
    .unwrap();
    std::fs::write(root.join("snapshots").join("keep.bin.zst"), b"data").unwrap();

    FileGraphStore::open(&root).await.unwrap();

    assert!(!root.join("manifest.json.tmp-1-abc").exists());
    assert!(!root.join("snapshots").join("x.bin.zst.tmp-9-def").exists());
    assert!(
        root.join("snapshots").join("keep.bin.zst").exists(),
        "only temp files are removed"
    );
    drop(dir);
}

#[tokio::test]
async fn hash_mismatch_marks_inconsistent() {
    let (dir, store, scope) = fresh_store(10).await;
    let root = store.root().to_path_buf();
    let f = fixture_of(store, scope);
    let ready = ready_full(&f, &fixture_graph(), sha('a').unwrap(), FINGERPRINT)
        .await
        .unwrap();

    let file = snapshot_files(&root).remove(0);
    let mut bytes = std::fs::read(&file).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&file, bytes).unwrap();

    let error = f.store.load_graph(&scope, ready.id).await.unwrap_err();
    assert!(
        matches!(error, StoreError::Integrity(_)),
        "a corrupted snapshot is an integrity error, got {error}"
    );
    let after = f.store.snapshot(&scope, ready.id).await.unwrap().unwrap();
    assert_eq!(after.status, SnapshotStatus::Inconsistent);
    assert!(matches!(
        f.store.load_graph(&scope, ready.id).await,
        Err(StoreError::InvalidStatus { .. })
    ));
    drop(dir);
}

#[tokio::test]
async fn corrupt_manifest_is_an_integrity_error() {
    let (dir, store, scope) = fresh_store(10).await;
    std::fs::write(store.root().join("manifest.json"), b"{ not json").unwrap();
    let error = store
        .create_snapshot(NewSnapshot::full(
            scope,
            sha('a').unwrap(),
            SnapshotPurpose::Local,
            FINGERPRINT,
        ))
        .await
        .unwrap_err();
    match error {
        StoreError::Integrity(message) => assert!(message.contains("review graph rebuild")),
        other => panic!("expected an integrity error, got {other}"),
    }
    drop(dir);
}

/// Helper process for `two_processes_serialize_writes`: holds the store lock until its stdin
/// closes. A no-op when run as an ordinary test.
#[test]
fn lock_holder_child() {
    let Ok(path) = std::env::var(CHILD_ENV) else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .unwrap();
    fs2::FileExt::lock_exclusive(&file).unwrap();
    println!("LOCKED");
    let mut sink = String::new();
    let _ = std::io::stdin().read_line(&mut sink);
}

#[tokio::test]
async fn two_processes_serialize_writes() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileGraphStore::open_with(
        dir.path().join("graph"),
        FileStoreOptions {
            lock_timeout: Duration::from_millis(400),
            ..FileStoreOptions::default()
        },
    )
    .await
    .unwrap();
    let scope = scope();
    let request = || {
        NewSnapshot::full(
            scope,
            sha('a').unwrap(),
            SnapshotPurpose::Local,
            FINGERPRINT,
        )
    };

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "lock_holder_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, store.root().join(".lock"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    loop {
        let line = lines.next().expect("the helper exited early").unwrap();
        if line.contains("LOCKED") {
            break;
        }
    }

    // Another process holds the exclusive lock: the mutation times out with a conflict.
    match store.create_snapshot(request()).await {
        Err(StoreError::Conflict(message)) => assert_eq!(message, "graph store locked"),
        other => panic!("expected a lock conflict, got {other:?}"),
    }

    // Releasing the lock lets the same write through.
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    store.create_snapshot(request()).await.unwrap();
}

#[tokio::test]
async fn retention_keeps_heads_and_chains() {
    let (dir, store, scope) = fresh_store(1).await;
    let root = store.root().to_path_buf();
    let f = fixture_of(store, scope);

    // A default-branch full snapshot and a pull-request delta on top of it.
    let full = ready_full(&f, &fixture_graph(), sha('a').unwrap(), FINGERPRINT)
        .await
        .unwrap();
    let pr = pending_delta(
        &f,
        sha('b').unwrap(),
        SnapshotPurpose::PullRequest,
        FINGERPRINT,
        full.id,
    )
    .await
    .unwrap();
    f.store
        .write_delta(&scope, pr.id, &GraphDelta::default())
        .await
        .unwrap();
    must_transition(&f, pr.id, SnapshotStatus::Persisting, SnapshotStatus::Ready)
        .await
        .unwrap();

    // Two newer default-branch snapshots move that head forward.
    let second = ready_full(&f, &fixture_graph(), sha('c').unwrap(), OTHER_FINGERPRINT)
        .await
        .unwrap();
    let third = ready_full(&f, &fixture_graph(), sha('d').unwrap(), FINGERPRINT)
        .await
        .unwrap();

    // `second` is neither the newest, a head, nor part of a kept chain: it is gone, file and all.
    assert!(f.store.snapshot(&scope, second.id).await.unwrap().is_none());
    // The pull-request head survives, and so does the base it depends on.
    assert!(f.store.snapshot(&scope, pr.id).await.unwrap().is_some());
    assert!(f.store.snapshot(&scope, full.id).await.unwrap().is_some());
    assert!(f.store.snapshot(&scope, third.id).await.unwrap().is_some());
    f.store.load_graph(&scope, pr.id).await.unwrap();
    assert_eq!(
        snapshot_files(&root).len(),
        3,
        "the deleted snapshot's file was removed"
    );
    drop(dir);
}

#[tokio::test]
async fn delta_chain_survives_reopening() {
    let (dir, store, scope) = fresh_store(10).await;
    let root = store.root().to_path_buf();
    let f = fixture_of(store, scope);
    let full = ready_full(&f, &fixture_graph(), sha('a').unwrap(), FINGERPRINT)
        .await
        .unwrap();
    let delta = ready_delta(
        &f,
        &GraphDelta::default(),
        sha('b').unwrap(),
        FINGERPRINT,
        full.id,
    )
    .await
    .unwrap();
    drop(f);

    let reopened = FileGraphStore::open(&root).await.unwrap();
    let chain = reopened.chain(&scope, delta.id).await.unwrap();
    assert_eq!(chain.len(), 2);
    let base_graph = reopened.load_graph(&scope, full.id).await.unwrap();
    assert!(!base_graph.nodes.is_empty());
    assert_eq!(
        reopened.load_graph(&scope, delta.id).await.unwrap(),
        base_graph,
        "an empty delta materializes to its base"
    );
    drop(dir);
}
