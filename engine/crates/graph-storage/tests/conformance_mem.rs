//! The GS-001 conformance suite against the in-memory reference adapter.
//!
//! Runs with `cargo test -p graph-storage --features conformance`.

#![cfg(feature = "conformance")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use async_trait::async_trait;
use graph_storage::conformance::{run_all, Harness, HarnessFixture};
use graph_storage::mem::MemGraphStore;
use graph_storage::StoreError;
use repository::store::RepoScope;
use review_core::ids::{OrganizationId, RepositoryId};

#[derive(Debug)]
struct MemHarness;

#[async_trait]
impl Harness for MemHarness {
    fn name(&self) -> &str {
        "mem"
    }

    async fn fresh(&self) -> Result<HarnessFixture, StoreError> {
        Ok(HarnessFixture {
            store: Arc::new(MemGraphStore::new()),
            scope: RepoScope {
                organization_id: OrganizationId::new(),
                repository_id: RepositoryId::new(),
            },
            foreign: RepoScope {
                organization_id: OrganizationId::new(),
                repository_id: RepositoryId::new(),
            },
        })
    }
}

#[tokio::test]
async fn conformance_mem() {
    let report = run_all(&MemHarness).await;
    assert_eq!(report.adapter, "mem");
    report.assert_ok();
}
