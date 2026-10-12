//! GS-007: compaction over the in-memory reference store. The Postgres variants (including the
//! concurrent-compaction race, which needs the ready-fingerprint index) are in
//! `pg_conformance.rs`.

#![cfg(feature = "conformance")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use graph_storage::compaction::{compact, should_compact, CompactionDecision, CompactionPolicy};
use graph_storage::conformance::{edge, fixture_graph, sha, with_file_versions, HarnessFixture};
use graph_storage::kinds::EdgeKind;
use graph_storage::mem::MemGraphStore;
use graph_storage::model::{flatten, GraphDelta};
use graph_storage::status::SnapshotStatus;
use graph_storage::types::{NewSnapshot, SnapshotKind, SnapshotPurpose};
use graph_storage::GraphStore;
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};

fn fixture() -> HarnessFixture {
    HarnessFixture {
        store: Arc::new(MemGraphStore::new()),
        scope: RepoScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        },
        foreign: RepoScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        },
    }
}

async fn persisting(f: &HarnessFixture, req: NewSnapshot) -> SnapshotId {
    let meta = f.store.create_snapshot(req).await.unwrap();
    for (from, to) in [
        (SnapshotStatus::Pending, SnapshotStatus::Indexing),
        (SnapshotStatus::Indexing, SnapshotStatus::Persisting),
    ] {
        assert!(f
            .store
            .transition(&f.scope, meta.id, from, to, None)
            .await
            .unwrap());
    }
    meta.id
}

async fn ready(f: &HarnessFixture, id: SnapshotId) {
    assert!(f
        .store
        .transition(
            &f.scope,
            id,
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready,
            None
        )
        .await
        .unwrap());
}

/// A full snapshot plus `n` small deltas; returns the head and the deltas.
async fn chain(f: &HarnessFixture, n: u8) -> (SnapshotId, Vec<GraphDelta>) {
    let base_g = with_file_versions(f, &fixture_graph()).await.unwrap();
    let base = persisting(
        f,
        NewSnapshot::full(
            f.scope,
            sha('a').unwrap(),
            SnapshotPurpose::DefaultBranch,
            [1; 32],
        ),
    )
    .await;
    f.store.write_full(&f.scope, base, &base_g).await.unwrap();
    ready(f, base).await;
    let mut head = base;
    let mut deltas = Vec::new();
    for i in 0..n {
        let d = GraphDelta {
            edges_added: vec![edge(2, EdgeKind::Calls, 4, 300 + u16::from(i))],
            ..GraphDelta::default()
        };
        let mut fingerprint = [2u8; 32];
        fingerprint[0] = i;
        let id = persisting(
            f,
            NewSnapshot::delta(
                f.scope,
                sha('b').unwrap(),
                SnapshotPurpose::DefaultBranch,
                head,
                fingerprint,
            ),
        )
        .await;
        f.store.write_delta(&f.scope, id, &d).await.unwrap();
        ready(f, id).await;
        head = id;
        deltas.push(d);
    }
    (head, deltas)
}

#[tokio::test]
async fn compacted_full_equals_chain_materialization() {
    let f = fixture();
    let (head, deltas) = chain(&f, 21).await;
    let metas = f.store.chain(&f.scope, head).await.unwrap();
    assert!(matches!(
        should_compact(&metas, &CompactionPolicy::default()),
        CompactionDecision::Compact { .. }
    ));

    let compacted = compact(f.store.as_ref(), &f.scope, head).await.unwrap();
    assert_eq!(compacted.kind, SnapshotKind::Full);
    assert_eq!(compacted.purpose, SnapshotPurpose::Compaction);
    assert_eq!(compacted.status, SnapshotStatus::Ready);
    assert_eq!(compacted.stats.compacted_from, Some(head));
    assert_eq!(compacted.stats.replaced_chain.len(), 22);

    let materialized = f.store.load_graph(&f.scope, head).await.unwrap();
    let rewritten = f.store.load_graph(&f.scope, compacted.id).await.unwrap();
    assert_eq!(rewritten, materialized);
    let base = f.store.load_graph(&f.scope, metas[0].id).await.unwrap();
    assert_eq!(rewritten, flatten(&base, &deltas));

    // A second call returns the same snapshot instead of writing another one.
    let again = compact(f.store.as_ref(), &f.scope, head).await.unwrap();
    assert_eq!(again.id, compacted.id);
}

#[tokio::test]
async fn compaction_failure_keeps_chain_readable() {
    let f = fixture();
    let (head, _) = chain(&f, 2).await;
    // A head that is not ready cannot be compacted; the chain it sits on is untouched.
    let pending = persisting(
        &f,
        NewSnapshot::delta(
            f.scope,
            sha('c').unwrap(),
            SnapshotPurpose::DefaultBranch,
            head,
            [9; 32],
        ),
    )
    .await;
    assert!(compact(f.store.as_ref(), &f.scope, pending).await.is_err());
    assert!(f.store.load_graph(&f.scope, head).await.is_ok());
    let metas = f.store.chain(&f.scope, head).await.unwrap();
    assert_eq!(metas.len(), 3);
}

#[tokio::test]
async fn next_delta_bases_on_compacted_full() {
    let f = fixture();
    let (head, _) = chain(&f, 3).await;
    let compacted = compact(f.store.as_ref(), &f.scope, head).await.unwrap();
    let d = GraphDelta {
        edges_added: vec![edge(3, EdgeKind::Calls, 4, 777)],
        ..GraphDelta::default()
    };
    let next = persisting(
        &f,
        NewSnapshot::delta(
            f.scope,
            sha('d').unwrap(),
            SnapshotPurpose::DefaultBranch,
            compacted.id,
            [5; 32],
        ),
    )
    .await;
    f.store.write_delta(&f.scope, next, &d).await.unwrap();
    ready(&f, next).await;
    let metas = f.store.chain(&f.scope, next).await.unwrap();
    assert_eq!(metas.len(), 2, "the new chain starts at the compacted full");
    let base = f.store.load_graph(&f.scope, compacted.id).await.unwrap();
    let loaded = f.store.load_graph(&f.scope, next).await.unwrap();
    assert_eq!(loaded, flatten(&base, std::slice::from_ref(&d)));
}
