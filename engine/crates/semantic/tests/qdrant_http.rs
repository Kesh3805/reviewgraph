#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-003: request shapes and error handling of the Qdrant client, through the public index.

mod common;

use std::sync::atomic::Ordering;

use common::{convention_units, fake_index, index_for, scope, snapshot, COLLECTION, DIMS};
use semantic::index::DeleteSelector;
use semantic::{CollectionTargets, Error, QdrantError, SemanticQuery, UnitKind};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn upsert_batches_of_256() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let units = convention_units(repo, snapshot(), 300);
    let report = t.index.upsert_units(&scope, &units).await.unwrap();
    assert_eq!(report.upserted, 300);
    let puts = fake.requests("PUT", &format!("/collections/{COLLECTION}/points"));
    let sizes: Vec<usize> = puts
        .iter()
        .map(|b| b["points"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, vec![256, 44]);
    assert_eq!(fake.points(COLLECTION).len(), 300);
    t.audit.assert_clean();
}

#[tokio::test]
async fn search_request_shape() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let snap = snapshot();
    t.index
        .upsert_units(&scope, &convention_units(repo, snap, 3))
        .await
        .unwrap();
    let mut q = SemanticQuery::text("dependency injection services", 5);
    q.kinds = vec![UnitKind::Convention];
    q.snapshot_id = Some(snap);
    q.min_score = 0.01;
    let hits = t.index.search(&scope, &q).await.unwrap();
    assert!(!hits.is_empty());
    assert!(hits
        .iter()
        .all(|h| h.kind == UnitKind::Convention && h.repository_id == repo));
    let body = fake
        .requests("POST", "/points/query")
        .pop()
        .expect("query request");
    assert_eq!(body["limit"], 5);
    assert_eq!(body["with_payload"], true);
    assert_eq!(body["query"].as_array().unwrap().len(), usize::from(DIMS));
    assert!((body["score_threshold"].as_f64().unwrap() - 0.01).abs() < 1e-6);
    assert_eq!(
        body["filter"],
        json!({"must": [
            {"key": "organization_id", "match": {"value": scope.organization_id().to_string()}},
            {"key": "repository_id", "match": {"any": [repo.to_string()]}},
            {"key": "kind", "match": {"any": ["convention"]}},
            {"key": "snapshot_ids", "match": {"value": snap.to_string()}}
        ]})
    );
    t.audit.assert_clean();
}

#[tokio::test]
async fn delete_by_filter_shape() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    t.index
        .upsert_units(&scope, &convention_units(repo, snapshot(), 2))
        .await
        .unwrap();
    t.index
        .delete_units(
            &scope,
            DeleteSelector::FilePaths(vec![common::path("src/a.ts")]),
        )
        .await
        .unwrap();
    let body = fake.requests("POST", "/points/delete").pop().unwrap();
    assert_eq!(
        body,
        json!({"filter": {"must": [
            {"key": "organization_id", "match": {"value": scope.organization_id().to_string()}},
            {"key": "repository_id", "match": {"any": [repo.to_string()]}},
            {"key": "file_path", "match": {"any": ["src/a.ts"]}}
        ]}})
    );
    // Nothing matched the path: both points remain.
    assert_eq!(fake.points(COLLECTION).len(), 2);
    t.index
        .delete_units(
            &scope,
            DeleteSelector::Units {
                kind: UnitKind::Convention,
                keys: vec!["conv-0".into()],
            },
        )
        .await
        .unwrap();
    assert_eq!(fake.points(COLLECTION).len(), 1);
    t.audit.assert_clean();
}

#[tokio::test]
async fn error_mapping_503_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let t = index_for(
        &server.uri(),
        DIMS,
        Some(CollectionTargets::single(COLLECTION)),
    );
    let (scope, _) = scope();
    let err = t
        .index
        .search(&scope, &SemanticQuery::text("x", 3))
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Qdrant(QdrantError::Unavailable(_))),
        "{err:?}"
    );
    // One attempt plus three retries.
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
async fn error_mapping_404_and_400() {
    let server = MockServer::start().await;
    Mock::given(path(format!("/collections/{COLLECTION}/points/query")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path(format!("/collections/{COLLECTION}/points/delete")))
        .respond_with(
            ResponseTemplate::new(400).set_body_string("{\"status\":{\"error\":\"bad\"}}"),
        )
        .mount(&server)
        .await;
    let t = index_for(
        &server.uri(),
        DIMS,
        Some(CollectionTargets::single(COLLECTION)),
    );
    let (scope, _) = scope();
    assert!(matches!(
        t.index.search(&scope, &SemanticQuery::text("x", 3)).await,
        Err(Error::Qdrant(QdrantError::NotFound))
    ));
    assert!(matches!(
        t.index.delete_units(&scope, DeleteSelector::All).await,
        Err(Error::Qdrant(QdrantError::BadRequest { .. }))
    ));
    // Not retried.
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn retry_then_success() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    t.index
        .upsert_units(&scope, &convention_units(repo, snapshot(), 1))
        .await
        .unwrap();
    fake.fail_next.store(2, Ordering::SeqCst);
    let hits = t
        .index
        .search(&scope, &SemanticQuery::text("services", 3))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let queries: Vec<Value> = fake.requests("POST", "/points/query");
    assert_eq!(queries.len(), 3);
}

#[tokio::test]
async fn search_limit_is_bounded() {
    let (_server, _fake, t) = fake_index().await;
    let (scope, _) = scope();
    for limit in [0, 101] {
        assert!(matches!(
            t.index
                .search(&scope, &SemanticQuery::text("x", limit))
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
}
