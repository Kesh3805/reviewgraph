//! Database migrations (DOM-009). `engine/migrations` is the only schema source for both
//! languages (ADR-014).

use std::collections::BTreeSet;

use anyhow::Context;
use sqlx::migrate::{Migrate, Migrator};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// The embedded, forward-only migrations. sqlx verifies the checksum of every applied
/// migration and serializes concurrent runs with a Postgres advisory lock.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Applies pending migrations and returns the versions applied by this call (empty when the
/// database was already up to date).
#[tracing::instrument(name = "db_migrate", skip(pool))]
pub async fn run(pool: &PgPool) -> anyhow::Result<Vec<i64>> {
    let before = applied_versions(pool).await?;
    MIGRATOR.run(pool).await.context("applying migrations")?;
    let after = applied_versions(pool).await?;
    let newly_applied: Vec<i64> = after.difference(&before).copied().collect();
    for version in &newly_applied {
        tracing::info!(version, "migration_applied");
    }
    Ok(newly_applied)
}

async fn applied_versions(pool: &PgPool) -> anyhow::Result<BTreeSet<i64>> {
    let mut conn = pool.acquire().await.context("connecting to the database")?;
    conn.ensure_migrations_table()
        .await
        .context("creating the migrations table")?;
    let applied = conn.list_applied_migrations().await?;
    Ok(applied.into_iter().map(|m| m.version).collect())
}

/// Connects to `database_url` and applies pending migrations.
pub async fn run_from_url(database_url: &str) -> anyhow::Result<Vec<i64>> {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .context("connecting to the database")?;
    let applied = run(&pool).await;
    pool.close().await;
    applied
}
