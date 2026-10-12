//! Where the engine's data comes from.
//!
//! [`EngineBackend`] is the seam between the HTTP layer and storage: the production
//! implementation ([`PgBackend`]) reads snapshots through `graph_storage::pg::PgGraphStore`,
//! impact graphs from `review_runs.provenance` and source text from a bare git mirror; tests plug
//! in an in-memory one. Every method takes the tenant scope taken from the verified token, and an
//! id outside that scope is [`BackendError::NotFound`], never an existence oracle.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use codegraph::Graph;
use graph_storage::pg::PgGraphStore;
use graph_storage::{GraphStore, StoreError};
use impact::ImpactGraph;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use sqlx::PgPool;
use uuid::Uuid;

use crate::convert;

/// Largest file the source route will read from the mirror.
pub const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// Unknown, foreign or not-yet-ready.
    #[error("not found")]
    NotFound,
    #[error("backend unavailable: {0}")]
    Unavailable(String),
}

#[async_trait]
pub trait EngineBackend: Send + Sync + std::fmt::Debug {
    /// The materialized, query-ready graph of a `Ready` snapshot.
    async fn load_graph(
        &self,
        scope: RepoScope,
        snapshot: SnapshotId,
    ) -> Result<Arc<Graph>, BackendError>;

    /// The commit a snapshot was built from.
    async fn snapshot_commit(
        &self,
        scope: RepoScope,
        snapshot: SnapshotId,
    ) -> Result<String, BackendError>;

    /// The impact graph recorded for a review run of this repository; `None` when the run has
    /// none (yet).
    async fn impact_graph(
        &self,
        scope: RepoScope,
        run: Uuid,
    ) -> Result<Option<ImpactGraph>, BackendError>;

    /// The text of `path` at `commit_sha`. `path` is already validated as repository-relative.
    async fn read_source(
        &self,
        scope: RepoScope,
        commit_sha: &str,
        path: &str,
    ) -> Result<String, BackendError>;
}

/// The Postgres and git-mirror backend.
#[derive(Debug, Clone)]
pub struct PgBackend {
    store: PgGraphStore,
    pool: PgPool,
    mirror_root: Option<PathBuf>,
}

impl PgBackend {
    /// `mirror_root` holds one bare mirror per repository, `{repository_id}.git`.
    pub fn new(pool: PgPool, mirror_root: Option<PathBuf>) -> Self {
        Self {
            store: PgGraphStore::new(pool.clone()),
            pool,
            mirror_root,
        }
    }
}

fn map_store(error: StoreError) -> BackendError {
    match error {
        // Not visible in this scope, or not readable yet: both look like "no such snapshot".
        StoreError::NotFound(_) | StoreError::InvalidStatus { .. } => BackendError::NotFound,
        other => BackendError::Unavailable(other.to_string()),
    }
}

#[async_trait]
impl EngineBackend for PgBackend {
    async fn load_graph(
        &self,
        scope: RepoScope,
        snapshot: SnapshotId,
    ) -> Result<Arc<Graph>, BackendError> {
        let stored = self
            .store
            .load_graph(&scope, snapshot)
            .await
            .map_err(map_store)?;
        let converted = tokio::task::spawn_blocking(move || convert::to_query_graph(&stored))
            .await
            .map_err(|e| BackendError::Unavailable(e.to_string()))?
            .map_err(BackendError::Unavailable)?;
        let (graph, stats) = converted;
        if stats.dropped_nodes + stats.dropped_edges > 0 {
            tracing::warn!(
                dropped_nodes = stats.dropped_nodes,
                dropped_edges = stats.dropped_edges,
                "snapshot rows the query layer does not understand were left out"
            );
        }
        Ok(Arc::new(graph))
    }

    async fn snapshot_commit(
        &self,
        scope: RepoScope,
        snapshot: SnapshotId,
    ) -> Result<String, BackendError> {
        let meta = self
            .store
            .snapshot(&scope, snapshot)
            .await
            .map_err(map_store)?
            .ok_or(BackendError::NotFound)?;
        Ok(meta.commit_sha.to_string())
    }

    async fn impact_graph(
        &self,
        scope: RepoScope,
        run: Uuid,
    ) -> Result<Option<ImpactGraph>, BackendError> {
        let unavailable = |e: sqlx::Error| BackendError::Unavailable(e.to_string());
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SELECT set_config('app.organization_id', $1, true)")
            .bind(scope.organization_id.as_uuid().to_string())
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let row: Option<(Option<serde_json::Value>,)> = sqlx::query_as(
            "SELECT provenance -> 'impact_graph' FROM review_runs \
              WHERE id = $1 AND organization_id = $2 AND repository_id = $3",
        )
        .bind(run)
        .bind(*scope.organization_id.as_uuid())
        .bind(*scope.repository_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        match row {
            None => Err(BackendError::NotFound),
            Some((None,)) => Ok(None),
            Some((Some(value),)) => serde_json::from_value::<ImpactGraph>(value)
                .map(Some)
                .map_err(|e| BackendError::Unavailable(format!("stored impact graph: {e}"))),
        }
    }

    async fn read_source(
        &self,
        scope: RepoScope,
        commit_sha: &str,
        path: &str,
    ) -> Result<String, BackendError> {
        let root = self.mirror_root.as_ref().ok_or_else(|| {
            BackendError::Unavailable("no source mirror is configured".to_owned())
        })?;
        let valid_sha = matches!(commit_sha.len(), 40 | 64)
            && commit_sha.bytes().all(|b| b.is_ascii_hexdigit());
        if !valid_sha {
            return Err(BackendError::NotFound);
        }
        let git_dir = root.join(format!("{}.git", scope.repository_id.as_uuid()));
        let output = tokio::process::Command::new("git")
            .arg("--git-dir")
            .arg(&git_dir)
            .arg("show")
            .arg(format!("{commit_sha}:{path}"))
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| BackendError::Unavailable(format!("git: {e}")))?;
        if !output.status.success() {
            return Err(BackendError::NotFound);
        }
        if output.stdout.len() > MAX_SOURCE_BYTES {
            return Err(BackendError::Unavailable("file is too large".to_owned()));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}
