//! POL-002 against PostgreSQL: `repository_configs` is content-addressed and idempotent.

#![cfg(feature = "integration")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::pg::{seed_scope, seed_snapshot, TestDb};
use profile::config::sync::{sync_config, MemoryTree};
use profile::config::CONFIG_PATH;
use profile::pg::PgConfigStore;

#[tokio::test]
async fn config_rows_are_content_addressed() {
    let db = TestDb::new().await;
    let scope = seed_scope(&db.pool).await;
    let store = PgConfigStore::new(db.pool.clone());

    let valid = sync_config(&MemoryTree::default().with(CONFIG_PATH, "version: 1\n"));
    assert!(store.save(&scope, &valid).await.unwrap());
    assert!(
        !store.save(&scope, &valid).await.unwrap(),
        "retry is a no-op"
    );

    let invalid = sync_config(&MemoryTree::default().with(CONFIG_PATH, "version: 1\nbogus: 1\n"));
    assert!(store.save(&scope, &invalid).await.unwrap());
    let row = store
        .get(&scope, &invalid.config_hash().to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "invalid");
    assert_eq!(row.validation[0]["path"], "bogus");

    let snapshot = seed_snapshot(&db.pool, &scope).await;
    store.bind_snapshot(&scope, snapshot, &valid).await.unwrap();
    let path: Option<String> =
        sqlx::query_scalar("SELECT config_source_path FROM snapshots WHERE id = $1")
            .bind(snapshot)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(path.as_deref(), Some(CONFIG_PATH));
    db.finish().await;
}
