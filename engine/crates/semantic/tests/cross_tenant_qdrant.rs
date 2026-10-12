#![cfg(feature = "integration")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEC-002 (Qdrant half): two organizations index an identical repository (identical symbols and
//! content hashes) into the same collection; every scoped read returns only the caller's points,
//! while an unscoped raw probe sees both (proving the test has teeth).

mod common;

use std::collections::BTreeSet;

use common::{
    authorize_symbol, function_symbol, live_index, snapshot, TestIndex, COLLECTION, DIMS,
};
use review_core::ids::{OrganizationId, RepositoryId};
use semantic::bench::SplitMix;
use semantic::sync::{sync, SyncOptions, SyncRequest};
use semantic::units::{code_chunks, symbol_summary, EmbeddingUnit, SymbolInput};
use semantic::{SemanticQuery, TenantScope};
use serde_json::{json, Value};

fn fixture() -> Vec<SymbolInput> {
    let mut symbols = vec![authorize_symbol()];
    for (name, file) in [
        ("PermissionService.check", "src/auth/permission.service.ts"),
        ("AdminService.updateUser", "src/admin/admin.service.ts"),
        ("UserController.update", "src/users/user.controller.ts"),
        ("ReportService.generate", "src/reports/report.service.ts"),
        ("authorizeHeader", "src/util/format.ts"),
    ] {
        symbols.push(function_symbol(name, file, 8));
    }
    symbols
}

fn units(repo: RepositoryId) -> Vec<EmbeddingUnit> {
    let snap = snapshot();
    fixture()
        .iter()
        .flat_map(|s| {
            let mut u: Vec<EmbeddingUnit> = symbol_summary(s, repo, snap).into_iter().collect();
            u.extend(code_chunks(s, repo, snap));
            u
        })
        .collect()
}

async fn index_tenant(t: &TestIndex, scope: &TenantScope, units: &[EmbeddingUnit]) {
    sync(
        &t.index,
        scope,
        SyncRequest {
            snapshot_id: snapshot(),
            changed: units,
            removed: &[],
            renamed: &[],
        },
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
}

struct Tenants {
    t: TestIndex,
    a: TenantScope,
    b: TenantScope,
    repo_a: RepositoryId,
    repo_b: RepositoryId,
    units_a: Vec<EmbeddingUnit>,
}

async fn setup() -> Option<Tenants> {
    let t = live_index().await?;
    let (repo_a, repo_b) = (RepositoryId::new(), RepositoryId::new());
    let a = TenantScope::single(OrganizationId::new(), repo_a);
    let b = TenantScope::single(OrganizationId::new(), repo_b);
    let (units_a, units_b) = (units(repo_a), units(repo_b));
    // Identical content in both tenants.
    let hashes = |u: &[EmbeddingUnit]| -> BTreeSet<String> {
        u.iter().map(|x| x.content_hash.to_string()).collect()
    };
    assert_eq!(hashes(&units_a), hashes(&units_b));
    // Index both concurrently to catch shared state between tenants.
    tokio::join!(
        index_tenant(&t, &a, &units_a),
        index_tenant(&t, &b, &units_b)
    );
    Some(Tenants {
        t,
        a,
        b,
        repo_a,
        repo_b,
        units_a,
    })
}

fn queries() -> Vec<SemanticQuery> {
    let mut out: Vec<SemanticQuery> = fixture()
        .iter()
        .map(|s| SemanticQuery::text(s.qualified_name.clone(), 50))
        .collect();
    let mut rng = SplitMix::new(2_002);
    while out.len() < 25 {
        let mut v: Vec<f32> = (0..DIMS)
            .map(|_| (rng.next_u64() % 2_001) as f32 / 1_000.0 - 1.0)
            .collect();
        semantic::embedding::l2_normalize(&mut v);
        out.push(SemanticQuery::vector(v, 50));
    }
    out
}

#[tokio::test]
async fn identical_repos_in_two_orgs_never_cross_hit_in_qdrant() {
    let Some(x) = setup().await else { return };
    let violations_before = semantic::metrics::scope_violations_total();
    for q in queries() {
        let hits_a = x.t.index.search(&x.a, &q).await.unwrap();
        assert!(!hits_a.is_empty());
        assert!(hits_a.iter().all(|h| h.repository_id == x.repo_a));
        let hits_b = x.t.index.search(&x.b, &q).await.unwrap();
        assert!(hits_b.iter().all(|h| h.repository_id == x.repo_b));
        // Same content, different points.
        let ids_a: BTreeSet<_> = hits_a.iter().map(|h| h.point_id).collect();
        assert!(hits_b.iter().all(|h| !ids_a.contains(&h.point_id)));
    }
    // The server itself never returned a foreign point (the post-check had nothing to drop).
    assert_eq!(
        semantic::metrics::scope_violations_total(),
        violations_before
    );
}

#[tokio::test]
async fn unscoped_raw_probe_returns_both_orgs_proving_test_sensitivity() {
    let Some(x) = setup().await else { return };
    let url = std::env::var("QDRANT_URL").unwrap();
    let vector = semantic::embedding::hash::embed_text(&x.units_a[0].text, DIMS);
    // A raw request without any filter, which the public API cannot express.
    let resp: Value = reqwest::Client::new()
        .post(format!("{url}/collections/{COLLECTION}/points/query"))
        .json(&json!({"query": vector, "limit": 100, "with_payload": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let orgs: BTreeSet<String> = resp["result"]["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["score"].as_f64().unwrap_or(0.0) > 0.999)
        .filter_map(|p| p["payload"]["organization_id"].as_str().map(str::to_owned))
        .collect();
    assert!(orgs.contains(&x.a.organization_id().to_string()));
    assert!(orgs.contains(&x.b.organization_id().to_string()));
}

#[tokio::test]
async fn all_recorded_qdrant_requests_carry_org_and_repo_filters() {
    let Some(x) = setup().await else { return };
    for q in queries().into_iter().take(5) {
        x.t.index.search(&x.a, &q).await.unwrap();
    }
    x.t.index
        .delete_units(&x.b, semantic::DeleteSelector::All)
        .await
        .unwrap();
    let records = x.t.audit.records();
    for op in ["upsert", "scroll", "search", "delete"] {
        assert!(records.iter().any(|r| r.op == op), "no {op} recorded");
    }
    x.t.audit.assert_clean();
    // Every upsert payload carries the tenant keys from the scope that wrote it.
    for r in records.iter().filter(|r| r.op == "upsert") {
        for p in r.body["points"].as_array().unwrap() {
            let org = p["payload"]["organization_id"].as_str().unwrap();
            let repo = p["payload"]["repository_id"].as_str().unwrap();
            let pair = (org.to_owned(), repo.to_owned());
            assert!(
                pair == (x.a.organization_id().to_string(), x.repo_a.to_string())
                    || pair == (x.b.organization_id().to_string(), x.repo_b.to_string()),
                "{pair:?}"
            );
        }
    }
}
