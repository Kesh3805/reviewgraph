#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-007: incremental embedding sync.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::{fake_index, function_symbol, path, scope, snapshot, COLLECTION, DIMS};
use review_core::ids::{RepositoryId, SnapshotId};
use semantic::embedding::{
    standard, EmbedError, EmbedRequest, EmbedResponse, EmbeddingProvider, EmbeddingSpace,
    HashProvider,
};
use semantic::sync::{gc, sync, Rekey, SyncOptions, SyncReport, SyncRequest, UnitRef};
use semantic::units::{code_chunks, symbol_summary, EmbeddingUnit};
use semantic::{
    CollectionRegistry, CollectionTargets, MemoryRegistry, QdrantConfig, SemanticIndex, TenantScope,
};
use serde_json::Value;

fn units_of(repo: RepositoryId, snap: SnapshotId, n: usize) -> Vec<EmbeddingUnit> {
    let mut out = Vec::new();
    for i in 0..n {
        let sym = function_symbol(&format!("handler{i}"), &format!("src/h{i}.ts"), 8);
        out.extend(symbol_summary(&sym, repo, snap));
        out.extend(code_chunks(&sym, repo, snap));
    }
    out
}

async fn run(
    index: &SemanticIndex,
    scope: &TenantScope,
    snap: SnapshotId,
    changed: &[EmbeddingUnit],
    opts: &SyncOptions,
) -> SyncReport {
    sync(
        index,
        scope,
        SyncRequest {
            snapshot_id: snap,
            changed,
            removed: &[],
            renamed: &[],
        },
        opts,
        None,
    )
    .await
    .unwrap()
}

fn snapshot_ids(payload: &serde_json::Map<String, Value>) -> Vec<String> {
    payload["snapshot_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn second_sync_embeds_nothing() {
    let (_server, _fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = units_of(repo, snapshot(), 5);
    let first = run(
        &t.index,
        &scope,
        snapshot(),
        &units,
        &SyncOptions::default(),
    )
    .await;
    assert_eq!(first.embedded, units.len() as u64);
    assert!(first.tokens > 0);
    t.provider.reset();
    let second = run(
        &t.index,
        &scope,
        snapshot(),
        &units,
        &SyncOptions::default(),
    )
    .await;
    assert_eq!(second.embedded, 0);
    assert_eq!(second.skipped_unchanged, units.len() as u64);
    assert_eq!(t.provider.calls(), 0);
    t.audit.assert_clean();
}

#[tokio::test]
async fn changed_body_reembedded() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let sym = function_symbol("checkAccess", "src/access.ts", 8);
    let mut units: Vec<EmbeddingUnit> =
        symbol_summary(&sym, repo, snapshot()).into_iter().collect();
    units.extend(code_chunks(&sym, repo, snapshot()));
    run(
        &t.index,
        &scope,
        snapshot(),
        &units,
        &SyncOptions::default(),
    )
    .await;
    let before = fake.points(COLLECTION);

    let mut changed = sym.clone();
    changed.body = Some(
        changed
            .body
            .unwrap()
            .replace("checkAccessHelper(3)", "allowAll()"),
    );
    let s2 = snapshot();
    let mut units2: Vec<EmbeddingUnit> = symbol_summary(&changed, repo, s2).into_iter().collect();
    units2.extend(code_chunks(&changed, repo, s2));
    t.provider.reset();
    let r = run(&t.index, &scope, s2, &units2, &SyncOptions::default()).await;
    // The summary has no body text: only the chunk is re-embedded.
    assert_eq!(r.embedded, 1);
    assert_eq!(r.skipped_unchanged, 1);
    assert_eq!(t.provider.texts(), 1);
    let after = fake.points(COLLECTION);
    assert_eq!(before.len(), after.len());
    let chunk_id = t.index.point_id_of(&scope, &units2[1]);
    assert_ne!(
        before[&chunk_id.to_string()].1["content_hash"],
        after[&chunk_id.to_string()].1["content_hash"]
    );
}

#[tokio::test]
async fn rename_unchanged_body_rekeyed_without_embedding() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = units_of(repo, snapshot(), 1);
    run(
        &t.index,
        &scope,
        snapshot(),
        &units,
        &SyncOptions::default(),
    )
    .await;
    let old = units[0].clone();
    let old_id = t.index.point_id_of(&scope, &old);
    let old_vector = fake.points(COLLECTION)[&old_id.to_string()].0.clone();

    // Same text under a new key (moved symbol, unchanged body).
    let mut moved = old.clone();
    moved.key = "f".repeat(32);
    moved.file_path = Some(path("src/moved/h0.ts"));
    let s2 = snapshot();
    moved.snapshot_id = s2;
    t.provider.reset();
    let r = sync(
        &t.index,
        &scope,
        SyncRequest {
            snapshot_id: s2,
            changed: &[],
            removed: &[],
            renamed: &[Rekey {
                from: UnitRef::from(&old),
                to: moved.clone(),
            }],
        },
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.rekeyed, 1);
    assert_eq!(r.embedded, 0);
    assert_eq!(t.provider.calls(), 0);
    let points = fake.points(COLLECTION);
    assert!(!points.contains_key(&old_id.to_string()));
    let new = &points[&t.index.point_id_of(&scope, &moved).to_string()];
    assert_eq!(new.0, old_vector);
    assert_eq!(new.1["file_path"], "src/moved/h0.ts");
    assert!(snapshot_ids(&new.1).contains(&s2.to_string()));
}

#[tokio::test]
async fn snapshot_ids_appended_not_reembedded() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = units_of(repo, snapshot(), 2);
    let (s1, s2) = (snapshot(), snapshot());
    run(&t.index, &scope, s1, &units, &SyncOptions::default()).await;
    t.provider.reset();
    run(&t.index, &scope, s2, &units, &SyncOptions::default()).await;
    assert_eq!(t.provider.calls(), 0);
    for (_, (_, payload)) in fake.points(COLLECTION) {
        assert_eq!(snapshot_ids(&payload), vec![s1.to_string(), s2.to_string()]);
    }
    // One grouped payload update for all points.
    assert_eq!(fake.requests("POST", "/points/payload").len(), 1);
}

#[tokio::test]
async fn gc_deletes_points_not_in_live_snapshots() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let (closed_pr, head) = (snapshot(), snapshot());
    let pr_units = units_of(repo, closed_pr, 2);
    let head_units: Vec<EmbeddingUnit> = units_of(repo, head, 4).split_off(4);
    run(
        &t.index,
        &scope,
        closed_pr,
        &pr_units,
        &SyncOptions::default(),
    )
    .await;
    run(&t.index, &scope, head, &head_units, &SyncOptions::default()).await;
    assert_eq!(
        fake.points(COLLECTION).len(),
        pr_units.len() + head_units.len()
    );
    let deleted = gc(&t.index, &scope, &[head]).await.unwrap();
    assert_eq!(deleted, pr_units.len() as u64);
    assert_eq!(fake.points(COLLECTION).len(), head_units.len());
    assert!(gc(&t.index, &scope, &[]).await.is_err());
    t.audit.assert_clean();
}

#[tokio::test]
async fn gc_keeps_default_branch_head() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = units_of(repo, snapshot(), 1);
    let head = snapshot();
    let opts = SyncOptions {
        snapshot_cap: 3,
        pinned_snapshots: vec![head],
        ..SyncOptions::default()
    };
    run(&t.index, &scope, head, &units, &opts).await;
    for _ in 0..5 {
        run(&t.index, &scope, snapshot(), &units, &opts).await;
    }
    for (_, (_, payload)) in fake.points(COLLECTION) {
        let ids = snapshot_ids(&payload);
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], head.to_string());
    }
    assert_eq!(gc(&t.index, &scope, &[head]).await.unwrap(), 0);
    assert_eq!(fake.points(COLLECTION).len(), units.len());
}

#[tokio::test]
async fn token_budget_defers_and_reports() {
    let (_server, _fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = units_of(repo, snapshot(), 6);
    let per_unit = semantic::embedding::estimate_tokens(&units[0].text) as u64;
    let tight = SyncOptions {
        token_budget: per_unit * 3,
        ..SyncOptions::default()
    };
    let first = run(&t.index, &scope, snapshot(), &units, &tight).await;
    assert!(first.deferred > 0, "{first:?}");
    assert_eq!(first.embedded + first.deferred, units.len() as u64);
    let second = run(
        &t.index,
        &scope,
        snapshot(),
        &units,
        &SyncOptions::default(),
    )
    .await;
    assert_eq!(second.embedded, first.deferred);
    assert_eq!(second.skipped_unchanged, first.embedded);
}

#[tokio::test]
async fn crash_midway_resume_idempotent() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = units_of(repo, snapshot(), 2);
    let opts = SyncOptions {
        batch: 2,
        ..SyncOptions::default()
    };
    // Requests: scroll, upsert (batch 1), then the next scroll fails.
    *fake.fail_after.lock().unwrap() = Some(fake.state.lock().unwrap().log.len() + 2);
    let crashed = sync(
        &t.index,
        &scope,
        SyncRequest {
            snapshot_id: snapshot(),
            changed: &units,
            removed: &[],
            renamed: &[],
        },
        &opts,
        None,
    )
    .await;
    assert!(crashed.is_err());
    assert_eq!(fake.points(COLLECTION).len(), 2);
    *fake.fail_after.lock().unwrap() = None;
    t.provider.reset();
    let resumed = run(&t.index, &scope, snapshot(), &units, &opts).await;
    assert_eq!(resumed.embedded, (units.len() - 2) as u64);
    assert_eq!(resumed.skipped_unchanged, 2);
    assert_eq!(t.provider.texts(), units.len() - 2);
    let again = run(&t.index, &scope, snapshot(), &units, &opts).await;
    assert_eq!(again.embedded, 0);
}

/// Records the maximum number of concurrent embedding calls.
#[derive(Debug)]
struct Slow {
    inner: HashProvider,
    in_flight: AtomicUsize,
    max: AtomicUsize,
}

#[async_trait]
impl EmbeddingProvider for Slow {
    fn space(&self) -> &EmbeddingSpace {
        self.inner.space()
    }

    fn max_batch(&self) -> usize {
        self.inner.max_batch()
    }

    fn max_input_tokens(&self) -> usize {
        self.inner.max_input_tokens()
    }

    async fn embed(&self, req: EmbedRequest<'_>) -> Result<EmbedResponse, EmbedError> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let out = self.inner.embed(req).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        out
    }
}

#[tokio::test]
async fn concurrent_sync_serialized_by_lock() {
    let (server, _fake, _t) = fake_index().await;
    let slow = Arc::new(Slow {
        inner: HashProvider::new(DIMS, 1).unwrap(),
        in_flight: AtomicUsize::new(0),
        max: AtomicUsize::new(0),
    });
    let index = SemanticIndex::new(
        QdrantConfig::new(server.uri()).without_backoff(),
        standard(Arc::clone(&slow)),
    )
    .unwrap()
    .with_targets(CollectionTargets::single(COLLECTION));
    let (scope, repo) = scope();
    let memory = MemoryRegistry::new();
    let registry: &dyn CollectionRegistry = &memory;
    let a = units_of(repo, snapshot(), 2);
    let b: Vec<EmbeddingUnit> = units_of(repo, snapshot(), 4).split_off(4);
    let opts = SyncOptions::default();
    let (ra, rb) = tokio::join!(
        sync(
            &index,
            &scope,
            SyncRequest {
                snapshot_id: snapshot(),
                changed: &a,
                removed: &[],
                renamed: &[],
            },
            &opts,
            Some(registry),
        ),
        sync(
            &index,
            &scope,
            SyncRequest {
                snapshot_id: snapshot(),
                changed: &b,
                removed: &[],
                renamed: &[],
            },
            &opts,
            Some(registry),
        )
    );
    assert!(ra.unwrap().embedded > 0);
    assert!(rb.unwrap().embedded > 0);
    assert_eq!(slow.max.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn removed_units_deleted() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = units_of(repo, snapshot(), 2);
    run(
        &t.index,
        &scope,
        snapshot(),
        &units,
        &SyncOptions::default(),
    )
    .await;
    let removed: Vec<UnitRef> = units.iter().take(1).map(UnitRef::from).collect();
    let r = sync(
        &t.index,
        &scope,
        SyncRequest {
            snapshot_id: snapshot(),
            changed: &[],
            removed: &removed,
            renamed: &[],
        },
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.deleted, 1);
    assert_eq!(fake.points(COLLECTION).len(), units.len() - 1);
}
