//! PROF-001: the RepositoryProfile model and its stores.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use profile::store::conformance::{run_conformance, sample_profile};
use profile::store::{FileProfileStore, MemoryProfileStore, ProfileStore};
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId, SnapshotId};

fn scope() -> RepoScope {
    RepoScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

#[tokio::test]
async fn profile_roundtrip_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileProfileStore::new(dir.path());
    let scope = scope();
    let profile = sample_profile(&scope, SnapshotId::new(), 3);
    assert!(store.save(&scope, &profile).await.unwrap().created);
    assert_eq!(store.get(&scope, &profile.key()).await.unwrap(), Some(profile.clone()));
    // A human-readable copy of the newest profile sits at .review/profile.json.
    let latest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join(".review/profile.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(latest["profile_version"], 3);
    // A fresh store over the same directory reads it back.
    let reopened = FileProfileStore::new(dir.path());
    assert_eq!(
        reopened.get(&scope, &profile.key()).await.unwrap(),
        Some(profile)
    );
}

#[tokio::test]
async fn profile_insert_conflict_is_noop() {
    let store = MemoryProfileStore::new();
    let scope = scope();
    let profile = sample_profile(&scope, SnapshotId::new(), 1);
    assert!(store.save(&scope, &profile).await.unwrap().created);
    let mut later = profile.clone();
    later.notes.clear();
    assert!(!store.save(&scope, &later).await.unwrap().created);
    assert_eq!(store.len(), 1);
    assert_eq!(store.get(&scope, &profile.key()).await.unwrap(), Some(profile));
}

#[tokio::test]
async fn profile_store_conformance_suite() {
    run_conformance(|| async { (MemoryProfileStore::new(), scope()) }).await;
    let dirs: std::sync::Mutex<Vec<tempfile::TempDir>> = std::sync::Mutex::new(Vec::new());
    run_conformance(|| {
        let dir = tempfile::tempdir().unwrap();
        let store = FileProfileStore::new(dir.path());
        dirs.lock().unwrap().push(dir);
        async move { (store, scope()) }
    })
    .await;
}

#[test]
fn profile_schema_exported() {
    let schema = serde_json::to_value(profile::model::json_schema()).unwrap();
    assert_eq!(schema["title"], "RepositoryProfile");
    assert_eq!(schema["additionalProperties"], serde_json::Value::Bool(false));
    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for field in [
        "repository_id",
        "snapshot_id",
        "profile_version",
        "config_hash",
        "architecture",
        "conventions",
        "testing",
        "api",
        "queues",
        "documentation_rules",
        "computed_at",
    ] {
        assert!(required.contains(&field), "{field}");
    }
    assert!(schema["definitions"]["Convention"].is_object());
}
