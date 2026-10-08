#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-005: tenant scope enforcement of the search and write API.

mod common;

use std::sync::atomic::Ordering;

use common::{convention_units, fake_index, scope, snapshot, COLLECTION, DIMS};
use review_core::ids::{OrganizationId, RepositoryId};
use semantic::index::DeleteSelector;
use semantic::sync::{sync, SyncOptions, SyncRequest};
use semantic::{Error, ExtraFilter, NonEmptyVec, SemanticQuery, TenantScope};
use serde_json::json;

#[tokio::test]
async fn every_request_has_org_and_repo_must_filter() {
    let (_server, _fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let snap = snapshot();
    let units = convention_units(repo, snap, 5);
    t.index.upsert_units(&scope, &units).await.unwrap();
    sync(
        &t.index,
        &scope,
        SyncRequest {
            snapshot_id: snapshot(),
            changed: &units,
            removed: &[],
            renamed: &[],
        },
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    t.index
        .search(&scope, &SemanticQuery::text("injection", 10))
        .await
        .unwrap();
    t.index.count(&scope, &ExtraFilter::new()).await.unwrap();
    t.index
        .delete_units(&scope, DeleteSelector::NotInSnapshots(vec![snap]))
        .await
        .unwrap();
    t.index
        .delete_units(&scope, DeleteSelector::All)
        .await
        .unwrap();
    for op in [
        "upsert",
        "search",
        "scroll",
        "set_payload",
        "count",
        "delete",
    ] {
        assert!(t.audit.count(op) > 0, "no {op} request was audited");
    }
    t.audit.assert_clean();
    for r in t.audit.records() {
        if semantic::audit::FILTERED_OPS.contains(&r.op) {
            let must = &r.body["filter"]["must"];
            assert_eq!(
                must[0],
                json!({"key": "organization_id", "match": {"value": scope.organization_id().to_string()}}),
                "{}",
                r.op
            );
            assert_eq!(must[1]["key"], "repository_id");
        }
    }
}

#[test]
fn audit_rejects_unscoped_bodies() {
    let audit = semantic::audit::TenantAudit::new();
    audit.observe("search", COLLECTION, &json!({"filter": {"must": []}}));
    audit.observe(
        "upsert",
        COLLECTION,
        &json!({"points": [{"id": "x", "payload": {}}]}),
    );
    assert_eq!(audit.violations().len(), 2);
}

#[tokio::test]
async fn write_payload_tenant_from_scope() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    t.index
        .upsert_units(&scope, &convention_units(repo, snapshot(), 1))
        .await
        .unwrap();
    let points = fake.points(COLLECTION);
    let (_, (_, payload)) = points.iter().next().unwrap();
    assert_eq!(
        payload["organization_id"],
        json!(scope.organization_id().to_string())
    );
    assert_eq!(payload["repository_id"], json!(repo.to_string()));
    assert_eq!(payload["kind"], json!("convention"));
    assert!(
        payload.get("text").is_none(),
        "unit text must not be stored"
    );
}

#[tokio::test]
async fn unit_outside_scope_rejected() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    let mut units = convention_units(repo, snapshot(), 2);
    units[1].repository_id = RepositoryId::new();
    let before = semantic::metrics::scope_violations_total();
    let err = t.index.upsert_units(&scope, &units).await.unwrap_err();
    assert!(matches!(err, Error::ScopeViolation(_)), "{err:?}");
    assert!(semantic::metrics::scope_violations_total() > before);
    // Nothing was written, not even the in-scope unit.
    assert!(fake.points(COLLECTION).is_empty());
    assert_eq!(t.provider.calls(), 0);
}

#[tokio::test]
async fn result_postcheck_drops_foreign_hits() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo) = scope();
    t.index
        .upsert_units(&scope, &convention_units(repo, snapshot(), 1))
        .await
        .unwrap();
    // A point of another tenant with an identical vector, returned by a misbehaving server.
    let (_, (vector, _)) = fake.points(COLLECTION).into_iter().next().unwrap();
    fake.insert_raw(
        COLLECTION,
        "00000000-0000-4000-8000-000000000001",
        vector,
        json!({
            "organization_id": OrganizationId::new().to_string(),
            "repository_id": repo.to_string(),
            "kind": "convention",
            "unit_key": "conv-0",
        }),
    );
    fake.ignore_search_filter.store(true, Ordering::SeqCst);
    let before = semantic::metrics::scope_violations_total();
    let hits = t
        .index
        .search(&scope, &SemanticQuery::text("services", 10))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits.iter().all(|h| h.repository_id == repo));
    assert!(semantic::metrics::scope_violations_total() > before);
}

#[tokio::test]
async fn multi_repository_scope_filters_any_of_its_repositories() {
    let (_server, _fake, t) = fake_index().await;
    let org = OrganizationId::new();
    let (a, b, c) = (
        RepositoryId::new(),
        RepositoryId::new(),
        RepositoryId::new(),
    );
    let all = TenantScope::new(org, NonEmptyVec::new(vec![a, b, c]).unwrap());
    for repo in [a, b, c] {
        t.index
            .upsert_units(&all, &convention_units(repo, snapshot(), 1))
            .await
            .unwrap();
    }
    let ab = TenantScope::new(org, NonEmptyVec::new(vec![a, b]).unwrap());
    let hits = t
        .index
        .search(&ab, &SemanticQuery::text("services", 10))
        .await
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| h.repository_id != c));
    let other_org = TenantScope::single(OrganizationId::new(), a);
    assert!(t
        .index
        .search(&other_org, &SemanticQuery::text("services", 10))
        .await
        .unwrap()
        .is_empty());
    let mut q = SemanticQuery::vector(vec![0.0; usize::from(DIMS) + 1], 5);
    q.min_score = 0.0;
    assert!(matches!(
        t.index.search(&ab, &q).await,
        Err(Error::InvalidInput(_))
    ));
    t.audit.assert_clean();
}
