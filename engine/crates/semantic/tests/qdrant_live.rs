#![cfg(feature = "integration")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-003 / SEM-005 / SEM-007 / SEM-009 against the live Qdrant of the CI integration job
//! (`QDRANT_URL`). Each test is a no-op without it.

mod common;

use common::{convention_units, live_index, scope, snapshot, units_for_live, DIMS};
use review_core::ids::OrganizationId;
use semantic::bench::{Bench, CorpusSpec};
use semantic::index::DeleteSelector;
use semantic::sync::{sync, SyncOptions, SyncRequest};
use semantic::{ExtraFilter, QdrantConfig, SemanticQuery, TenantScope};

#[tokio::test]
async fn live_roundtrip_upsert_search_delete() {
    let Some(t) = live_index().await else { return };
    let (scope, repo) = scope();
    let units = convention_units(repo, snapshot(), 5);
    t.index.upsert_units(&scope, &units).await.unwrap();
    assert_eq!(t.index.count(&scope, &ExtraFilter::new()).await.unwrap(), 5);
    let hits = t
        .index
        .search(&scope, &SemanticQuery::text(units[2].text.clone(), 3))
        .await
        .unwrap();
    assert_eq!(hits[0].key, "conv-2");
    assert!(hits[0].score > 0.99, "{}", hits[0].score);
    t.index
        .delete_units(&scope, DeleteSelector::All)
        .await
        .unwrap();
    assert_eq!(t.index.count(&scope, &ExtraFilter::new()).await.unwrap(), 0);
    t.audit.assert_clean();
}

#[tokio::test]
async fn two_org_isolation_live() {
    let Some(t) = live_index().await else { return };
    let (a, repo_a) = scope();
    let (b, repo_b) = scope();
    t.index
        .upsert_units(&a, &convention_units(repo_a, snapshot(), 3))
        .await
        .unwrap();
    t.index
        .upsert_units(&b, &convention_units(repo_b, snapshot(), 3))
        .await
        .unwrap();
    let hits_a = t
        .index
        .search(&a, &SemanticQuery::text("dependency injection", 50))
        .await
        .unwrap();
    assert_eq!(hits_a.len(), 3);
    assert!(hits_a.iter().all(|h| h.repository_id == repo_a));
    // Same repository id under a different organization sees nothing.
    let spoof = TenantScope::single(OrganizationId::new(), repo_a);
    assert!(t
        .index
        .search(&spoof, &SemanticQuery::text("dependency injection", 50))
        .await
        .unwrap()
        .is_empty());
    t.audit.assert_clean();
}

#[tokio::test]
async fn live_second_sync_embeds_nothing() {
    let Some(t) = live_index().await else { return };
    let (scope, repo) = scope();
    let units = units_for_live(repo, 50);
    let req = |snap| SyncRequest {
        snapshot_id: snap,
        changed: &units,
        removed: &[],
        renamed: &[],
    };
    let first = sync(
        &t.index,
        &scope,
        req(snapshot()),
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(first.embedded, units.len() as u64);
    t.provider.reset();
    let started = std::time::Instant::now();
    let second = sync(
        &t.index,
        &scope,
        req(snapshot()),
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(second.embedded, 0);
    assert_eq!(t.provider.calls(), 0);
    println!(
        "live unchanged re-sync of {} units: {:?}",
        units.len(),
        started.elapsed()
    );
    t.index
        .delete_units(&scope, DeleteSelector::All)
        .await
        .unwrap();
    t.audit.assert_clean();
}

#[tokio::test]
async fn semantic_benchmark_small_report() {
    let Some(cfg) = QdrantConfig::from_lookup(|k| std::env::var(k).ok()) else {
        return;
    };
    let mut spec = CorpusSpec::small();
    spec.dims = DIMS;
    let report = Bench::new(cfg, "rg_bench_ci_small")
        .unwrap()
        .run(&spec)
        .await
        .unwrap();
    println!(
        "SEMANTIC_BENCH_REPORT_BEGIN\n{}\nSEMANTIC_BENCH_REPORT_END",
        report.markdown()
    );
    println!(
        "SEMANTIC_BENCH_JSON {}",
        serde_json::to_string(&report).unwrap()
    );
    assert!(report.results.iter().all(|r| r.filtered_points > 0));
}
