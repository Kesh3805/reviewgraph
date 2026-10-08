#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! SEM-008: sync driven by invalidation sets.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{fake_index, function_symbol, path, scope, snapshot, COLLECTION};
use review_core::ids::{RepositoryId, SnapshotId, SymbolKey};
use review_core::location::RepoPath;
use semantic::invalidation::{
    apply, plan, DependentEdge, InvalidationInput, SemanticSyncJob, SyncedVersions, UnitSource,
};
use semantic::sync::{gc, SyncOptions};
use semantic::units::{ConventionInput, DocInput, SymbolInput};
use serde_json::json;

/// A repository of `n` functions where function `i` calls function `i + 1`.
struct Repo {
    symbols: BTreeMap<SymbolKey, SymbolInput>,
    docs: Vec<DocInput>,
}

impl Repo {
    fn new(n: usize) -> Self {
        let mut list: Vec<SymbolInput> = (0..n)
            .map(|i| function_symbol(&format!("step{i}"), &format!("src/step{i}.ts"), 6))
            .collect();
        for i in 0..n {
            if i + 1 < n {
                let callee = list[i + 1].qualified_name.clone();
                list[i].callees.push((callee, 0.9));
            }
            if i > 0 {
                let caller = list[i - 1].qualified_name.clone();
                list[i].callers.push(caller);
            }
        }
        Self {
            symbols: list.into_iter().map(|s| (s.symbol_key(), s)).collect(),
            docs: vec![DocInput {
                path: path("README.md"),
                text: "# Steps\nThe pipeline runs step0 to stepN.\n".into(),
            }],
        }
    }

    fn key(&self, name: &str) -> SymbolKey {
        *self
            .symbols
            .iter()
            .find(|(_, s)| s.qualified_name == name)
            .unwrap()
            .0
    }
}

impl UnitSource for Repo {
    fn symbols(&self, keys: &BTreeSet<SymbolKey>) -> Vec<SymbolInput> {
        keys.iter()
            .filter_map(|k| self.symbols.get(k).cloned())
            .collect()
    }

    fn all_symbols(&self) -> Vec<SymbolInput> {
        self.symbols.values().cloned().collect()
    }

    fn docs(&self, paths: &BTreeSet<RepoPath>) -> Vec<DocInput> {
        self.docs
            .iter()
            .filter(|d| paths.contains(&d.path))
            .cloned()
            .collect()
    }

    fn all_docs(&self) -> Vec<DocInput> {
        self.docs.clone()
    }

    fn conventions(&self) -> Vec<ConventionInput> {
        vec![ConventionInput {
            id: "steps-chain".into(),
            rule: "Each step calls the next".into(),
            scope: "src/**".into(),
            examples: vec!["step0".into()],
            confidence: 0.7,
        }]
    }
}

fn runtime() -> SyncedVersions {
    SyncedVersions::current("hash-fh256-256")
}

async fn full_sync(
    t: &common::TestIndex,
    scope: &semantic::TenantScope,
    repo_id: RepositoryId,
    repo: &Repo,
    snap: SnapshotId,
) -> semantic::SyncReport {
    let p = plan(&InvalidationInput::default(), None, &runtime());
    assert!(p.full);
    apply(
        &t.index,
        scope,
        repo_id,
        snap,
        &p,
        repo,
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn one_file_change_resyncs_only_its_units_and_dependents() {
    let (_server, _fake, t) = fake_index().await;
    let (scope, repo_id) = scope();
    let mut repo = Repo::new(20);
    let full = full_sync(&t, &scope, repo_id, &repo, snapshot()).await;
    // 20 summaries + 20 chunks + 1 doc + 1 convention.
    assert_eq!(full.embedded, 42);

    // step5's body changes; step4 calls it.
    let k5 = repo.key("step5");
    let k4 = repo.key("step4");
    let s5 = repo.symbols.get_mut(&k5).unwrap();
    s5.body = Some(
        s5.body
            .clone()
            .unwrap()
            .replace("step5Helper(2)", "skipChecks()"),
    );
    let inv = InvalidationInput {
        changed_symbols: vec![k5],
        dependents: vec![(k4, DependentEdge::Calls)],
        ..InvalidationInput::default()
    };
    let p = plan(&inv, Some(&runtime()), &runtime());
    assert!(!p.full);
    t.provider.reset();
    let r = apply(
        &t.index,
        &scope,
        repo_id,
        snapshot(),
        &p,
        &repo,
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    // step5: summary (unchanged) + chunk (changed); step4: summary only (unchanged).
    assert_eq!(r.skipped_unchanged + r.embedded, 3);
    assert_eq!(r.embedded, 1);
    assert!(t.provider.texts() <= 30);
    t.audit.assert_clean();
}

#[tokio::test]
async fn dependent_summary_unchanged_hash_not_reembedded() {
    let (_server, _fake, t) = fake_index().await;
    let (scope, repo_id) = scope();
    let repo = Repo::new(3);
    full_sync(&t, &scope, repo_id, &repo, snapshot()).await;
    let inv = InvalidationInput {
        dependents: vec![
            (repo.key("step0"), DependentEdge::Calls),
            (repo.key("step2"), DependentEdge::Implements),
        ],
        ..InvalidationInput::default()
    };
    let p = plan(&inv, Some(&runtime()), &runtime());
    t.provider.reset();
    let r = apply(
        &t.index,
        &scope,
        repo_id,
        snapshot(),
        &p,
        &repo,
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(r.embedded, 0);
    assert_eq!(r.skipped_unchanged, 2);
    assert_eq!(t.provider.calls(), 0);
}

#[tokio::test]
async fn removed_symbol_points_deleted() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo_id) = scope();
    let repo = Repo::new(3);
    full_sync(&t, &scope, repo_id, &repo, snapshot()).await;
    let k1 = repo.key("step1");
    let before = fake.points(COLLECTION).len();
    let inv = InvalidationInput {
        removed_symbols: vec![k1],
        ..InvalidationInput::default()
    };
    let p = plan(&inv, Some(&runtime()), &runtime());
    apply(
        &t.index,
        &scope,
        repo_id,
        snapshot(),
        &p,
        &repo,
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    let after = fake.points(COLLECTION);
    // Summary and chunk of step1 are gone.
    assert_eq!(after.len(), before - 2);
    assert!(after
        .values()
        .all(|(_, p)| p.get("symbol_key") != Some(&json!(k1.to_string()))));
}

#[tokio::test]
async fn template_version_bump_triggers_full_resync() {
    let now = runtime();
    let older = SyncedVersions {
        template_version: now.template_version - 1,
        ..now.clone()
    };
    let other_space = SyncedVersions::current("openai-text_embedding_3_small-1536");
    let inv = InvalidationInput::default();
    assert!(plan(&inv, Some(&older), &now).full);
    assert!(plan(&inv, Some(&other_space), &now).full);
    let p = plan(&inv, Some(&now), &now);
    assert!(!p.full && p.symbols_full.is_empty() && !p.conventions);

    // A full plan rebuilds every unit; hashes decide what is embedded.
    let (_server, _fake, t) = fake_index().await;
    let (scope, repo_id) = scope();
    let repo = Repo::new(4);
    let first = full_sync(&t, &scope, repo_id, &repo, snapshot()).await;
    let second = full_sync(&t, &scope, repo_id, &repo, snapshot()).await;
    assert_eq!(second.skipped_unchanged, first.embedded);
    assert_eq!(second.embedded, 0);
}

#[tokio::test]
async fn pr_snapshot_units_tagged_and_gced_on_close() {
    let (_server, fake, t) = fake_index().await;
    let (scope, repo_id) = scope();
    let mut repo = Repo::new(3);
    let head = snapshot();
    full_sync(&t, &scope, repo_id, &repo, head).await;
    let base_points = fake.points(COLLECTION).len();

    // A pull request adds one function.
    let added = function_symbol("prOnly", "src/pr.ts", 6);
    let added_key = added.symbol_key();
    repo.symbols.insert(added_key, added);
    let pr = snapshot();
    let inv = InvalidationInput {
        added_symbols: vec![added_key],
        ..InvalidationInput::default()
    };
    let p = plan(&inv, Some(&runtime()), &runtime());
    apply(
        &t.index,
        &scope,
        repo_id,
        pr,
        &p,
        &repo,
        &SyncOptions::default(),
        None,
    )
    .await
    .unwrap();
    let points = fake.points(COLLECTION);
    assert_eq!(points.len(), base_points + 2);
    let pr_tagged = points
        .values()
        .filter(|(_, p)| p["snapshot_ids"] == json!([pr.to_string()]))
        .count();
    assert_eq!(pr_tagged, 2);
    // The pull request closes: only the default-branch head stays live.
    let deleted = gc(&t.index, &scope, &[head]).await.unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(fake.points(COLLECTION).len(), base_points);
}

#[test]
fn job_payload_ids_only() {
    let job = SemanticSyncJob {
        repository_id: RepositoryId::new(),
        snapshot_id: SnapshotId::new(),
        invalidation_ref: "inv:42".into(),
    };
    let v = serde_json::to_value(&job).unwrap();
    let keys: BTreeSet<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        BTreeSet::from(["repository_id", "snapshot_id", "invalidation_ref"])
    );
    assert!(serde_json::from_value::<SemanticSyncJob>(json!({
        "repository_id": job.repository_id, "snapshot_id": job.snapshot_id,
        "invalidation_ref": "x", "units": []
    }))
    .is_err());
    assert_eq!(
        job.idempotency_key("hash-fh256-256"),
        format!(
            "semsync:{}:{}:hash-fh256-256",
            job.repository_id, job.snapshot_id
        )
    );
}
