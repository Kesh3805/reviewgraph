//! PostgreSQL adapters (feature `pg`).
//!
//! Every statement runs inside a transaction that first sets `app.organization_id`
//! (`set_config(..., true)`), so row-level security applies, and repeats the `organization_id`
//! predicate as defence in depth. Parameters are always bound. Nothing here logs content: only
//! ids and hashes.

pub mod config;

use repository::store::RepoScope;
use sqlx::{PgPool, Postgres, Transaction};

pub use config::PgConfigStore;

/// Opens a transaction scoped to the tenant of `scope`.
pub(crate) async fn begin_scoped<'a>(
    pool: &'a PgPool,
    scope: &RepoScope,
) -> Result<Transaction<'a, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.organization_id', $1, true)")
        .bind(scope.organization_id.as_uuid().to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// Errors of the PostgreSQL adapters.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PgStoreError {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("stored row is invalid: {0}")]
    Corrupt(String),
}
