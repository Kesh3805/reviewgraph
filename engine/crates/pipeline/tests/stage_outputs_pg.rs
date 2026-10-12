//! stage_outputs and run claims against Postgres (PIPE-005). Run with `--features integration`.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;

use pipeline::store::{
    metrics, MemoryBlobStore, NewRun, NewStageOutput, PgStageStore, PutOutcome, RunClaims,
    StageStatus, StageStore, REUSABLE_ACROSS_RETRY, SPILL_THRESHOLD_BYTES,
};
use review_core::ids::{OrganizationId, ReviewRunId};
use review_core::review::ReviewTrigger;
use serde_json::json;
use support::{seed_run, TestDb};

fn output(stage: &str, hash: &str, v: serde_json::Value) -> NewStageOutput {
    NewStageOutput {
        stage: stage.to_owned(),
        input_hash: hash.to_owned(),
        status: StageStatus::Succeeded,
        schema_version: 1,
        duration_ms: 5,
        output: v,
    }
}

fn h(c: char) -> String {
    c.to_string().repeat(64)
}

#[tokio::test]
async fn put_get_roundtrip() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"b".repeat(40)).await;
    let (org, run) = (
        OrganizationId::from_uuid(s.org),
        ReviewRunId::from_uuid(s.run),
    );
    let store = PgStageStore::new(db.pool.clone(), Arc::new(MemoryBlobStore::new()));
    let put = store
        .put(
            org,
            run,
            output("diff", &h('a'), json!({"files": [1, 2, 3]})),
        )
        .await
        .unwrap();
    assert!(matches!(put, PutOutcome::Stored(_)));
    let got = store.get(org, run, "diff", &h('a')).await.unwrap().unwrap();
    assert_eq!(got.output, json!({"files": [1, 2, 3]}));
    assert_eq!(got.status, StageStatus::Succeeded);
    assert!(store
        .get(org, run, "diff", &h('b'))
        .await
        .unwrap()
        .is_none());
    assert_eq!(store.list_for_run(org, run).await.unwrap().len(), 1);
    db.finish().await;
}

#[tokio::test]
async fn second_put_same_key_keeps_first_and_reports_hash_mismatch() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"b".repeat(40)).await;
    let (org, run) = (
        OrganizationId::from_uuid(s.org),
        ReviewRunId::from_uuid(s.run),
    );
    let store = PgStageStore::new(db.pool.clone(), Arc::new(MemoryBlobStore::new()));
    store
        .put(org, run, output("impact", &h('a'), json!({"v": 1})))
        .await
        .unwrap();
    let before = metrics::counts("impact").conflicts;
    let second = store
        .put(org, run, output("impact", &h('a'), json!({"v": 2})))
        .await
        .unwrap();
    match second {
        PutOutcome::Existing {
            stored,
            hash_matches,
        } => {
            assert!(!hash_matches);
            assert_eq!(stored.output, json!({"v": 1}));
        }
        PutOutcome::Stored(_) => panic!("must not overwrite"),
    }
    assert_eq!(metrics::counts("impact").conflicts, before + 1);
    // An identical repeat is not a conflict.
    let same = store
        .put(org, run, output("impact", &h('a'), json!({"v": 1})))
        .await
        .unwrap();
    assert!(matches!(
        same,
        PutOutcome::Existing {
            hash_matches: true,
            ..
        }
    ));
    db.finish().await;
}

#[tokio::test]
async fn large_output_spills_to_blob_and_reads_back() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"b".repeat(40)).await;
    let (org, run) = (
        OrganizationId::from_uuid(s.org),
        ReviewRunId::from_uuid(s.run),
    );
    let blobs = Arc::new(MemoryBlobStore::new());
    let store = PgStageStore::new(db.pool.clone(), blobs.clone());
    let big = json!({ "text": "x".repeat(SPILL_THRESHOLD_BYTES + 1024) });
    store
        .put(org, run, output("context", &h('c'), big.clone()))
        .await
        .unwrap();
    assert_eq!(blobs.len(), 1);
    let (inline, key): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT output::text, blob_key FROM stage_outputs WHERE stage = 'context'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(inline.is_none() && key.is_some());
    let got = store
        .get(org, run, "context", &h('c'))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.output, big);
    // A lost blob is a cache miss, not an error.
    blobs.remove(&key.unwrap());
    assert!(store
        .get(org, run, "context", &h('c'))
        .await
        .unwrap()
        .is_none());
    db.finish().await;
}

#[tokio::test]
async fn resume_skips_completed_stage() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"b".repeat(40)).await;
    let (org, run) = (
        OrganizationId::from_uuid(s.org),
        ReviewRunId::from_uuid(s.run),
    );
    let store = PgStageStore::new(db.pool.clone(), Arc::new(MemoryBlobStore::new()));
    store
        .put(
            org,
            run,
            output("resume_index", &h('d'), json!({"ok": true})),
        )
        .await
        .unwrap();
    let before = metrics::counts("resume_index").hits;
    assert!(store
        .get(org, run, "resume_index", &h('d'))
        .await
        .unwrap()
        .is_some());
    assert_eq!(metrics::counts("resume_index").hits, before + 1);
    db.finish().await;
}

#[tokio::test]
async fn check_constraint_rejects_both_output_and_blob() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"b".repeat(40)).await;
    let err = sqlx::query(
        "INSERT INTO stage_outputs
           (review_run_id, organization_id, stage, input_hash, output_hash, output, blob_key,
            status, schema_version, duration_ms)
         VALUES ($1, $2, 'x', $3, $3, '{}'::jsonb, 'k', 'succeeded', 1, 1)",
    )
    .bind(s.run)
    .bind(s.org)
    .bind(h('a'))
    .execute(&db.pool)
    .await;
    assert!(err.is_err());
    db.finish().await;
}

#[tokio::test]
async fn other_tenant_cannot_read_outputs() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"b".repeat(40)).await;
    let other = seed_run(&db.pool, &"c".repeat(40)).await;
    let run = ReviewRunId::from_uuid(s.run);
    let store = PgStageStore::new(db.pool.clone(), Arc::new(MemoryBlobStore::new()));
    store
        .put(
            OrganizationId::from_uuid(s.org),
            run,
            output("diff", &h('a'), json!({})),
        )
        .await
        .unwrap();
    let got = store
        .get(OrganizationId::from_uuid(other.org), run, "diff", &h('a'))
        .await
        .unwrap();
    assert!(got.is_none());
    db.finish().await;
}

fn new_run(s: &support::SeededRun, head: &str) -> NewRun {
    NewRun {
        id: ReviewRunId::new(),
        organization_id: OrganizationId::from_uuid(s.org),
        repository_id: s.repository,
        pull_request_id: s.pull_request,
        base_sha: "a".repeat(40),
        head_sha: head.to_owned(),
        merge_base_sha: None,
        trigger: ReviewTrigger::Webhook,
        trace_parent: None,
    }
}

#[tokio::test]
async fn get_or_create_run_returns_same_run_for_same_head() {
    let db = TestDb::new().await;
    let head = "d".repeat(40);
    let s = seed_run(&db.pool, &head).await;
    let claims = RunClaims::new(db.pool.clone());
    // The seeded run already owns (pull request, head): a duplicate event returns it.
    let (run, created) = claims.get_or_create_run(new_run(&s, &head)).await.unwrap();
    assert!(!created);
    assert_eq!(*run.id.as_uuid(), s.run);
    db.finish().await;
}

#[tokio::test]
async fn retry_requires_failed_run_and_sets_retry_of() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"e".repeat(40)).await;
    let claims = RunClaims::new(db.pool.clone());
    let org = OrganizationId::from_uuid(s.org);
    let failed = ReviewRunId::from_uuid(s.run);
    // A RECEIVED run cannot be retried.
    assert!(claims
        .retry_run(org, failed, ReviewRunId::new())
        .await
        .is_err());
    sqlx::query(
        "UPDATE review_runs SET state = 'FAILED_INDEXING', failure_class = 'permanent',
                completed_at = now() WHERE id = $1",
    )
    .bind(s.run)
    .execute(&db.pool)
    .await
    .unwrap();
    let new_id = ReviewRunId::new();
    let retry = claims.retry_run(org, failed, new_id).await.unwrap();
    assert_eq!(retry.id, new_id);
    assert_eq!(retry.retry_of, Some(failed));
    db.finish().await;
}

#[tokio::test]
async fn retry_copies_only_reusable_outputs() {
    let db = TestDb::new().await;
    let s = seed_run(&db.pool, &"f".repeat(40)).await;
    let claims = RunClaims::new(db.pool.clone());
    let org = OrganizationId::from_uuid(s.org);
    let failed = ReviewRunId::from_uuid(s.run);
    let store = PgStageStore::new(db.pool.clone(), Arc::new(MemoryBlobStore::new()));
    store
        .put(org, failed, output("index", &h('1'), json!({"i": 1})))
        .await
        .unwrap();
    store
        .put(org, failed, output("review", &h('2'), json!({"r": 1})))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE review_runs SET state = 'FAILED_REVIEW', failure_class = 'permanent',
                completed_at = now() WHERE id = $1",
    )
    .bind(s.run)
    .execute(&db.pool)
    .await
    .unwrap();
    let retry = claims
        .retry_run(org, failed, ReviewRunId::new())
        .await
        .unwrap();
    let copied = claims
        .copy_reusable_outputs(org, failed, retry.id, REUSABLE_ACROSS_RETRY)
        .await
        .unwrap();
    assert_eq!(copied, 1);
    let rows = store.list_for_run(org, retry.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].stage, "index");
    db.finish().await;
}
