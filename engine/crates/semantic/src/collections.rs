//! Collection bootstrap, registry and cut-over (SEM-004, ADR-008).
//!
//! One Qdrant collection per embedding space and epoch (`rg_{space}_v{n}`). The PostgreSQL table
//! `semantic_collections` records each collection's state:
//! `building → active → retiring → retired`. At most one collection is active (partial unique
//! index). Searches read the active collection; writers write every non-retired collection of
//! their space (dual-write while a new epoch is building).

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::Instrument;

use crate::embedding::EmbeddingSpace;
use crate::error::{Error, Result};
use crate::filter::fields;
use crate::index::{CollectionTargets, SemanticIndex};
use crate::metrics;
use crate::qdrant::{HnswCfg, QdrantClient, QdrantConfig};

/// Advisory lock key that serializes bootstrap across workers.
pub const BOOTSTRAP_LOCK: &str = "semantic_bootstrap";
/// Default delay before a retiring collection is deleted.
pub const RETIRE_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);
/// Bootstrap retry interval while Qdrant is unavailable.
pub const BOOTSTRAP_RETRY: Duration = Duration::from_secs(60);

/// Lifecycle state of a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollectionStatus {
    Building,
    Active,
    Retiring,
    Retired,
}

impl CollectionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Active => "active",
            Self::Retiring => "retiring",
            Self::Retired => "retired",
        }
    }
}

impl fmt::Display for CollectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CollectionStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        [Self::Building, Self::Active, Self::Retiring, Self::Retired]
            .into_iter()
            .find(|c| c.as_str() == s)
            .ok_or_else(|| Error::Registry(format!("unknown collection state {s:?}")))
    }
}

/// One registry row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRecord {
    pub name: String,
    pub space_id: String,
    pub provider: String,
    pub model: String,
    pub dims: u16,
    pub version: u16,
    pub state: CollectionStatus,
}

impl CollectionRecord {
    pub fn for_space(space: &EmbeddingSpace, state: CollectionStatus) -> Self {
        Self {
            name: space.collection_name(),
            space_id: space.id(),
            provider: space.provider.as_str().to_owned(),
            model: space.model.clone(),
            dims: space.dims,
            version: space.version,
            state,
        }
    }
}

/// A held registry lock; released on drop.
pub struct RegistryLock(#[allow(dead_code)] Box<dyn Send>);

impl RegistryLock {
    pub fn new(guard: Box<dyn Send>) -> Self {
        Self(guard)
    }
}

impl fmt::Debug for RegistryLock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegistryLock")
    }
}

/// Collection registry port. [`MemoryRegistry`] for tests and single-process tools,
/// `PgRegistry` (feature `pg`) in production.
#[async_trait]
pub trait CollectionRegistry: Send + Sync + fmt::Debug {
    /// Takes a named mutual-exclusion lock (PostgreSQL advisory lock).
    async fn lock(&self, key: &str) -> Result<RegistryLock>;
    /// Every row, oldest first.
    async fn list(&self) -> Result<Vec<CollectionRecord>>;
    /// Inserts `rec` unless a row with its name exists; returns the stored row and whether it
    /// was inserted.
    async fn insert_if_absent(&self, rec: &CollectionRecord) -> Result<(CollectionRecord, bool)>;
    /// `building → active` and the previous `active → retiring`, atomically. Returns the
    /// previously active collection.
    async fn activate(&self, name: &str) -> Result<Option<String>>;
    /// Retiring collections whose state changed more than `older_than` ago.
    async fn retiring_older_than(&self, older_than: Duration) -> Result<Vec<CollectionRecord>>;
    async fn mark_retired(&self, name: &str) -> Result<()>;
}

#[derive(Debug, Default)]
struct MemoryState {
    rows: Vec<(CollectionRecord, Instant)>,
}

/// In-process registry with the same invariants as the PostgreSQL one.
#[derive(Debug, Default)]
pub struct MemoryRegistry {
    state: Mutex<MemoryState>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl MemoryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut MemoryState) -> Result<T>) -> Result<T> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| Error::Registry("registry mutex poisoned".into()))?;
        f(&mut guard)
    }
}

#[async_trait]
impl CollectionRegistry for MemoryRegistry {
    async fn lock(&self, key: &str) -> Result<RegistryLock> {
        let m = {
            let mut locks = self
                .locks
                .lock()
                .map_err(|_| Error::Registry("lock table poisoned".into()))?;
            locks.entry(key.to_owned()).or_default().clone()
        };
        Ok(RegistryLock::new(Box::new(m.lock_owned().await)))
    }

    async fn list(&self) -> Result<Vec<CollectionRecord>> {
        self.with_state(|s| Ok(s.rows.iter().map(|(r, _)| r.clone()).collect()))
    }

    async fn insert_if_absent(&self, rec: &CollectionRecord) -> Result<(CollectionRecord, bool)> {
        self.with_state(|s| {
            if let Some((existing, _)) = s.rows.iter().find(|(r, _)| r.name == rec.name) {
                return Ok((existing.clone(), false));
            }
            if rec.state == CollectionStatus::Active
                && s.rows
                    .iter()
                    .any(|(r, _)| r.state == CollectionStatus::Active)
            {
                return Err(Error::Registry(
                    "another collection is already active".into(),
                ));
            }
            s.rows.push((rec.clone(), Instant::now()));
            Ok((rec.clone(), true))
        })
    }

    async fn activate(&self, name: &str) -> Result<Option<String>> {
        self.with_state(|s| {
            let target = s
                .rows
                .iter()
                .position(|(r, _)| r.name == name)
                .ok_or_else(|| Error::Registry(format!("unknown collection {name}")))?;
            if s.rows[target].0.state != CollectionStatus::Building {
                return Err(Error::Registry(format!(
                    "collection {name} is {}, not building",
                    s.rows[target].0.state
                )));
            }
            let mut previous = None;
            for (r, changed) in s.rows.iter_mut() {
                if r.state == CollectionStatus::Active {
                    r.state = CollectionStatus::Retiring;
                    *changed = Instant::now();
                    previous = Some(r.name.clone());
                }
            }
            if let Some((r, changed)) = s.rows.get_mut(target) {
                r.state = CollectionStatus::Active;
                *changed = Instant::now();
            }
            Ok(previous)
        })
    }

    async fn retiring_older_than(&self, older_than: Duration) -> Result<Vec<CollectionRecord>> {
        self.with_state(|s| {
            Ok(s.rows
                .iter()
                .filter(|(r, changed)| {
                    r.state == CollectionStatus::Retiring && changed.elapsed() >= older_than
                })
                .map(|(r, _)| r.clone())
                .collect())
        })
    }

    async fn mark_retired(&self, name: &str) -> Result<()> {
        self.with_state(|s| {
            for (r, changed) in s.rows.iter_mut() {
                if r.name == name {
                    r.state = CollectionStatus::Retired;
                    *changed = Instant::now();
                }
            }
            Ok(())
        })
    }
}

/// Read and write targets for an index of `space`, given the registry rows.
///
/// Writes go to every active or building collection of the same space id (same model and
/// dims, any epoch), so vectors can be reused. Reads go to the active collection when it belongs
/// to this space, otherwise to the space's own collection: during a cross-model migration the
/// worker keeps the old index for reads until the new collection is activated.
pub fn targets_for(space: &EmbeddingSpace, rows: &[CollectionRecord]) -> CollectionTargets {
    let own = space.collection_name();
    let space_id = space.id();
    let mut write: Vec<String> = rows
        .iter()
        .filter(|r| {
            r.space_id == space_id
                && matches!(
                    r.state,
                    CollectionStatus::Active | CollectionStatus::Building
                )
        })
        .map(|r| r.name.clone())
        .collect();
    if !write.contains(&own) {
        write.push(own.clone());
    }
    write.sort();
    let read = rows
        .iter()
        .find(|r| r.state == CollectionStatus::Active && r.space_id == space_id)
        .map_or(own, |r| r.name.clone());
    CollectionTargets { read, write }
}

/// Outcome of a successful bootstrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapReport {
    pub collection: String,
    /// The Qdrant collection was created by this call.
    pub created: bool,
    /// Payload indexes created by this call.
    pub indexes_created: usize,
    pub state: CollectionStatus,
    pub targets: CollectionTargets,
}

async fn bootstrap_inner(
    index: &SemanticIndex,
    registry: &dyn CollectionRegistry,
    hnsw: HnswCfg,
) -> Result<BootstrapReport> {
    let _lock = registry.lock(BOOTSTRAP_LOCK).await?;
    let space = index.space().clone();
    let name = space.collection_name();
    let client = index.client();
    let state = client.ensure_collection(&name, space.dims, hnsw).await?;
    if state.dims != u64::from(space.dims) {
        return Err(Error::SpaceMismatch {
            collection: name,
            expected: space.dims,
            actual: state.dims,
        });
    }
    let mut indexes_created = 0;
    for (field, schema) in fields::INDEXED {
        if state.indexed_fields.iter().any(|f| f == field) {
            continue;
        }
        client
            .ensure_payload_index(&name, field, schema, field == fields::ORGANIZATION_ID)
            .await?;
        indexes_created += 1;
    }
    let rows = registry.list().await?;
    let initial = if rows.iter().any(|r| r.state == CollectionStatus::Active) {
        CollectionStatus::Building
    } else {
        // Nothing to migrate from: the first collection serves at once.
        CollectionStatus::Active
    };
    let (row, inserted) = registry
        .insert_if_absent(&CollectionRecord::for_space(&space, initial))
        .await?;
    if inserted {
        metrics::collection_transition(row.state.as_str());
    }
    let targets = targets_for(&space, &registry.list().await?);
    index.set_targets(targets.clone());
    Ok(BootstrapReport {
        collection: name,
        created: state.created,
        indexes_created,
        state: row.state,
        targets,
    })
}

/// Idempotent bootstrap for the index's space: collection (dims checked), payload indexes,
/// registry row, and the index's targets. Serialized across workers by [`BOOTSTRAP_LOCK`].
/// Sets the `semantic_available` gauge.
pub async fn bootstrap(
    index: &SemanticIndex,
    registry: &dyn CollectionRegistry,
    hnsw: HnswCfg,
) -> Result<BootstrapReport> {
    let space = index.space();
    let span = tracing::info_span!(
        "semantic_bootstrap",
        space = %space,
        collection = %space.collection_name()
    );
    let result = bootstrap_inner(index, registry, hnsw)
        .instrument(span)
        .await;
    metrics::set_available(result.is_ok());
    if let Err(e) = &result {
        tracing::warn!(error = %e, "semantic bootstrap failed; semantic retrieval disabled");
    }
    result
}

/// Retries [`bootstrap`] every `interval` while it fails with a retryable error (Qdrant or the
/// database unavailable). Reviews run without semantic context meanwhile. Returns the report
/// and the number of attempts.
pub async fn bootstrap_until_ready(
    index: &SemanticIndex,
    registry: &dyn CollectionRegistry,
    hnsw: HnswCfg,
    interval: Duration,
) -> Result<(BootstrapReport, u32)> {
    use review_core::Classify;
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match bootstrap(index, registry, hnsw).await {
            Ok(report) => return Ok((report, attempts)),
            Err(e) if e.class().is_retryable() => tokio::time::sleep(interval).await,
            Err(e) => return Err(e),
        }
    }
}

/// Re-reads the registry and updates the index's targets (after an activation elsewhere).
pub async fn refresh_targets(
    index: &SemanticIndex,
    registry: &dyn CollectionRegistry,
) -> Result<CollectionTargets> {
    let targets = targets_for(index.space(), &registry.list().await?);
    index.set_targets(targets.clone());
    Ok(targets)
}

/// `review semantic activate <name>`: flips `building → active`, previous `active → retiring`.
pub async fn activate(registry: &dyn CollectionRegistry, name: &str) -> Result<Option<String>> {
    let previous = registry.activate(name).await?;
    metrics::collection_transition(CollectionStatus::Active.as_str());
    if previous.is_some() {
        metrics::collection_transition(CollectionStatus::Retiring.as_str());
    }
    tracing::info!(collection = name, previous = ?previous, "semantic collection activated");
    Ok(previous)
}

/// Activates `name` when sync coverage reaches 99% of active units.
pub async fn activate_if_covered(
    registry: &dyn CollectionRegistry,
    name: &str,
    synced_units: u64,
    active_units: u64,
) -> Result<bool> {
    let covered =
        active_units == 0 || synced_units.saturating_mul(100) >= active_units.saturating_mul(99);
    let building = registry
        .list()
        .await?
        .iter()
        .any(|r| r.name == name && r.state == CollectionStatus::Building);
    if covered && building {
        activate(registry, name).await?;
        return Ok(true);
    }
    Ok(false)
}

/// `review semantic retire --older-than 7d`: deletes retiring collections from Qdrant and marks
/// them retired. Returns the retired names.
pub async fn retire(
    cfg: &QdrantConfig,
    registry: &dyn CollectionRegistry,
    older_than: Duration,
) -> Result<Vec<String>> {
    let client = QdrantClient::new(cfg.clone())?;
    let mut retired = Vec::new();
    for rec in registry.retiring_older_than(older_than).await? {
        client.delete_collection(&rec.name).await?;
        registry.mark_retired(&rec.name).await?;
        metrics::collection_transition(CollectionStatus::Retired.as_str());
        retired.push(rec.name);
    }
    Ok(retired)
}

#[cfg(feature = "pg")]
pub use pg::PgRegistry;

#[cfg(feature = "pg")]
mod pg {
    use std::time::Duration;

    use async_trait::async_trait;
    use sqlx::{PgPool, Row};

    use super::{CollectionRecord, CollectionRegistry, RegistryLock};
    use crate::error::{Error, Result};

    /// `semantic_collections` in PostgreSQL. Global table: collections are shared and tenant
    /// isolation is by payload filter, so the table holds no tenant data and has no RLS.
    #[derive(Debug, Clone)]
    pub struct PgRegistry {
        pool: PgPool,
    }

    impl PgRegistry {
        pub fn new(pool: PgPool) -> Self {
            Self { pool }
        }
    }

    fn record(row: &sqlx::postgres::PgRow) -> Result<CollectionRecord> {
        let dims: i32 = row.try_get("dims")?;
        let version: i32 = row.try_get("version")?;
        let state: String = row.try_get("state")?;
        Ok(CollectionRecord {
            name: row.try_get("name")?,
            space_id: row.try_get("space_id")?,
            provider: row.try_get("provider")?,
            model: row.try_get("model")?,
            dims: u16::try_from(dims).map_err(|_| Error::Registry(format!("dims {dims}")))?,
            version: u16::try_from(version)
                .map_err(|_| Error::Registry(format!("version {version}")))?,
            state: state.parse()?,
        })
    }

    const COLUMNS: &str = "name, space_id, provider, model, dims, version, state";

    #[async_trait]
    impl CollectionRegistry for PgRegistry {
        async fn lock(&self, key: &str) -> Result<RegistryLock> {
            let mut conn = self.pool.acquire().await?;
            // Closing the session on drop releases the advisory lock even if the holder
            // panics or forgets to unlock.
            conn.close_on_drop();
            sqlx::query("SELECT pg_advisory_lock(hashtext($1))")
                .bind(key)
                .execute(&mut *conn)
                .await?;
            Ok(RegistryLock::new(Box::new(conn)))
        }

        async fn list(&self) -> Result<Vec<CollectionRecord>> {
            let rows = sqlx::query(&format!(
                "SELECT {COLUMNS} FROM semantic_collections ORDER BY created_at, name"
            ))
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(record).collect()
        }

        async fn insert_if_absent(
            &self,
            rec: &CollectionRecord,
        ) -> Result<(CollectionRecord, bool)> {
            let inserted = sqlx::query(
                "INSERT INTO semantic_collections
                   (name, space_id, provider, model, dims, version, state, activated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7,
                         CASE WHEN $7 = 'active' THEN now() END)
                 ON CONFLICT (name) DO NOTHING",
            )
            .bind(&rec.name)
            .bind(&rec.space_id)
            .bind(&rec.provider)
            .bind(&rec.model)
            .bind(i32::from(rec.dims))
            .bind(i32::from(rec.version))
            .bind(rec.state.as_str())
            .execute(&self.pool)
            .await?
            .rows_affected()
                == 1;
            let row = sqlx::query(&format!(
                "SELECT {COLUMNS} FROM semantic_collections WHERE name = $1"
            ))
            .bind(&rec.name)
            .fetch_one(&self.pool)
            .await?;
            Ok((record(&row)?, inserted))
        }

        async fn activate(&self, name: &str) -> Result<Option<String>> {
            let mut tx = self.pool.begin().await?;
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM semantic_collections WHERE name = $1 FOR UPDATE",
            )
            .bind(name)
            .fetch_optional(&mut *tx)
            .await?;
            match state.as_deref() {
                Some("building") => {}
                Some(other) => {
                    return Err(Error::Registry(format!(
                        "collection {name} is {other}, not building"
                    )))
                }
                None => return Err(Error::Registry(format!("unknown collection {name}"))),
            }
            let previous: Option<String> = sqlx::query_scalar(
                "UPDATE semantic_collections SET state = 'retiring', state_changed_at = now()
                 WHERE state = 'active' RETURNING name",
            )
            .fetch_optional(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE semantic_collections
                 SET state = 'active', activated_at = now(), state_changed_at = now()
                 WHERE name = $1",
            )
            .bind(name)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(previous)
        }

        async fn retiring_older_than(&self, older_than: Duration) -> Result<Vec<CollectionRecord>> {
            let rows = sqlx::query(&format!(
                "SELECT {COLUMNS} FROM semantic_collections
                 WHERE state = 'retiring' AND state_changed_at <= now() - make_interval(secs => $1)
                 ORDER BY name"
            ))
            .bind(older_than.as_secs_f64())
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(record).collect()
        }

        async fn mark_retired(&self, name: &str) -> Result<()> {
            sqlx::query(
                "UPDATE semantic_collections SET state = 'retired', state_changed_at = now()
                 WHERE name = $1 AND state = 'retiring'",
            )
            .bind(name)
            .execute(&self.pool)
            .await?;
            Ok(())
        }
    }
}

/// Parses `7d`, `12h`, `30m`, `45s`.
pub fn parse_age(s: &str) -> Result<Duration> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: u64 = num
        .parse()
        .map_err(|_| Error::InvalidInput(format!("invalid age {s:?}")))?;
    let secs = match unit {
        "d" => n.saturating_mul(86_400),
        "h" => n.saturating_mul(3_600),
        "m" => n.saturating_mul(60),
        "s" => n,
        _ => return Err(Error::InvalidInput(format!("invalid age unit in {s:?}"))),
    };
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::ProviderName;

    fn space(version: u16) -> EmbeddingSpace {
        EmbeddingSpace::new(ProviderName::Hash, "fh768", 768, version).unwrap()
    }

    #[test]
    fn targets_dual_write_same_space() {
        let rows = vec![
            CollectionRecord::for_space(&space(1), CollectionStatus::Active),
            CollectionRecord::for_space(&space(2), CollectionStatus::Building),
        ];
        let t = targets_for(&space(2), &rows);
        assert_eq!(t.read, "rg_hash_fh768_768_v1");
        assert_eq!(
            t.write,
            vec![
                "rg_hash_fh768_768_v1".to_owned(),
                "rg_hash_fh768_768_v2".to_owned()
            ]
        );
    }

    #[test]
    fn ages_parse() {
        assert_eq!(parse_age("7d").unwrap(), Duration::from_secs(604_800));
        assert_eq!(parse_age("2h").unwrap(), Duration::from_secs(7_200));
        assert!(parse_age("7w").is_err());
        assert!(parse_age("").is_err());
    }

    #[tokio::test]
    async fn memory_registry_enforces_one_active() {
        let r = MemoryRegistry::new();
        r.insert_if_absent(&CollectionRecord::for_space(
            &space(1),
            CollectionStatus::Active,
        ))
        .await
        .unwrap();
        assert!(r
            .insert_if_absent(&CollectionRecord::for_space(
                &space(2),
                CollectionStatus::Active
            ))
            .await
            .is_err());
        r.insert_if_absent(&CollectionRecord::for_space(
            &space(2),
            CollectionStatus::Building,
        ))
        .await
        .unwrap();
        let prev = activate(&r, "rg_hash_fh768_768_v2").await.unwrap();
        assert_eq!(prev.as_deref(), Some("rg_hash_fh768_768_v1"));
        let states: Vec<_> = r
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.state)
            .collect();
        assert_eq!(
            states,
            vec![CollectionStatus::Retiring, CollectionStatus::Active]
        );
        assert!(activate(&r, "rg_hash_fh768_768_v2").await.is_err());
    }
}
