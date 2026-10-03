#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use repository::store::conformance::run_conformance;
use repository::store::{FileFactsStore, RepoScope};
use review_core::ids::{OrganizationId, RepositoryId};

#[tokio::test]
async fn file_store_passes_conformance() {
    // Keep every temp dir alive for the duration of the run.
    let dirs = std::sync::Mutex::new(Vec::new());
    run_conformance(|| {
        let dir = tempfile::tempdir().unwrap();
        let store = FileFactsStore::new(dir.path());
        dirs.lock().unwrap().push(dir);
        let scope = RepoScope {
            organization_id: OrganizationId::new(),
            repository_id: RepositoryId::new(),
        };
        async move { (store, scope) }
    })
    .await;
}

#[tokio::test]
async fn file_store_rejects_nil_scope() {
    use repository::store::RepositoryFactsStore;
    let dir = tempfile::tempdir().unwrap();
    let store = FileFactsStore::new(dir.path());
    let scope = RepoScope {
        organization_id: OrganizationId::from_uuid(uuid::Uuid::nil()),
        repository_id: RepositoryId::new(),
    };
    assert!(store.latest(&scope).await.is_err());
}
