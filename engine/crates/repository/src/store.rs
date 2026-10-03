//! `RepositoryFactsStore`: where init facts are persisted (INIT-013).
//!
//! The port lives here; the file adapter wraps `.review/repository.json` and the PostgreSQL
//! adapter lives in `graph-storage`. Both must pass [`conformance::run_conformance`].

use std::future::Future;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use review_core::ids::{CommitSha, OrganizationId, RepositoryId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::facts::{RepositoryFacts, REPOSITORY_FACTS_SCHEMA};
use crate::review_dir;

/// Maximum serialized size of one facts document.
pub const MAX_FACTS_BYTES: usize = 1024 * 1024;

/// Which repository of which tenant the facts belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoScope {
    pub organization_id: OrganizationId,
    pub repository_id: RepositoryId,
}

/// Identity of one stored facts document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FactsId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFacts {
    pub id: FactsId,
    /// False when an identical row (same repository, commit and `facts_hash`) already existed.
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredFacts {
    pub id: FactsId,
    pub facts: RepositoryFacts,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FactsStoreError {
    #[error("facts are {bytes} bytes; the limit is {limit}")]
    TooLarge { bytes: usize, limit: usize },
    #[error("facts schema version {version} is not supported")]
    SchemaUnsupported { version: u32 },
    #[error("facts carry no commit to key them by")]
    MissingCommit,
    #[error("conflicting concurrent write")]
    Conflict,
    #[error("store backend: {0}")]
    Backend(String),
}

/// Persistence of `RepositoryFacts`, tenant-scoped and versioned per commit.
pub trait RepositoryFactsStore: Send + Sync {
    /// Idempotent on `(repository, commit, facts_hash)`.
    fn save(
        &self,
        scope: &RepoScope,
        facts: &RepositoryFacts,
    ) -> impl Future<Output = Result<SavedFacts, FactsStoreError>> + Send;

    /// The facts with the newest `detected_at`.
    fn latest(
        &self,
        scope: &RepoScope,
    ) -> impl Future<Output = Result<Option<StoredFacts>, FactsStoreError>> + Send;

    fn by_commit(
        &self,
        scope: &RepoScope,
        commit_sha: &CommitSha,
    ) -> impl Future<Output = Result<Option<StoredFacts>, FactsStoreError>> + Send;
}

/// Checks every store applies before writing: schema version, commit key and size.
pub fn validate_for_save(facts: &RepositoryFacts) -> Result<(CommitSha, Vec<u8>), FactsStoreError> {
    if facts.schema_version > REPOSITORY_FACTS_SCHEMA {
        return Err(FactsStoreError::SchemaUnsupported {
            version: facts.schema_version,
        });
    }
    let sha = facts
        .git
        .as_ref()
        .and_then(|g| g.head.sha())
        .cloned()
        .ok_or(FactsStoreError::MissingCommit)?;
    let bytes = serde_json::to_vec(facts).map_err(|e| FactsStoreError::Backend(e.to_string()))?;
    if bytes.len() > MAX_FACTS_BYTES {
        return Err(FactsStoreError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_FACTS_BYTES,
        });
    }
    Ok((sha, bytes))
}

pub fn parse_detected_at(facts: &RepositoryFacts) -> Result<DateTime<Utc>, FactsStoreError> {
    DateTime::parse_from_rfc3339(&facts.detected_at)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| FactsStoreError::Backend(format!("detected_at: {e}")))
}

fn facts_id_of(facts: &RepositoryFacts, commit: &CommitSha) -> FactsId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(commit.as_str().as_bytes());
    hasher.update(facts.facts_hash.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    FactsId(Uuid::from_bytes(bytes))
}

/// Store over `.review/repository.json`: the latest facts only. The scope is only checked for
/// being a real (non-nil) scope.
#[derive(Debug, Clone)]
pub struct FileFactsStore {
    root: PathBuf,
}

impl FileFactsStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn check_scope(scope: &RepoScope) -> Result<(), FactsStoreError> {
        if scope.organization_id.as_uuid().is_nil() || scope.repository_id.as_uuid().is_nil() {
            return Err(FactsStoreError::Backend("scope must not be nil".to_owned()));
        }
        Ok(())
    }

    fn read(&self) -> Result<Option<(StoredFacts, CommitSha)>, FactsStoreError> {
        let Some(json) = review_dir::read_facts_json(&self.root) else {
            return Ok(None);
        };
        let facts: RepositoryFacts = serde_json::from_str(&json)
            .map_err(|e| FactsStoreError::Backend(format!("repository.json: {e}")))?;
        if facts.schema_version > REPOSITORY_FACTS_SCHEMA {
            return Err(FactsStoreError::SchemaUnsupported {
                version: facts.schema_version,
            });
        }
        let Some(sha) = facts.git.as_ref().and_then(|g| g.head.sha()).cloned() else {
            return Ok(None);
        };
        let created_at = parse_detected_at(&facts)?;
        Ok(Some((
            StoredFacts {
                id: facts_id_of(&facts, &sha),
                facts,
                created_at,
            },
            sha,
        )))
    }
}

impl RepositoryFactsStore for FileFactsStore {
    async fn save(
        &self,
        scope: &RepoScope,
        facts: &RepositoryFacts,
    ) -> Result<SavedFacts, FactsStoreError> {
        Self::check_scope(scope)?;
        let (sha, _bytes) = validate_for_save(facts)?;
        let detected_at = parse_detected_at(facts)?;
        let id = facts_id_of(facts, &sha);
        if let Some((existing, existing_sha)) = self.read()? {
            if existing_sha == sha && existing.facts.facts_hash == facts.facts_hash {
                return Ok(SavedFacts { id, created: false });
            }
            // A late-finishing older init must not replace newer facts.
            if parse_detected_at(&existing.facts)? > detected_at {
                return Ok(SavedFacts { id, created: true });
            }
        }
        let dir = review_dir::ensure_layout(&self.root)
            .map_err(|e| FactsStoreError::Backend(e.to_string()))?;
        let json = facts
            .to_pretty_json()
            .map_err(|e| FactsStoreError::Backend(e.to_string()))?;
        review_dir::write_facts_atomic(&dir, &json)
            .map_err(|e| FactsStoreError::Backend(e.to_string()))?;
        Ok(SavedFacts { id, created: true })
    }

    async fn latest(&self, scope: &RepoScope) -> Result<Option<StoredFacts>, FactsStoreError> {
        Self::check_scope(scope)?;
        Ok(self.read()?.map(|(stored, _)| stored))
    }

    async fn by_commit(
        &self,
        scope: &RepoScope,
        commit_sha: &CommitSha,
    ) -> Result<Option<StoredFacts>, FactsStoreError> {
        Self::check_scope(scope)?;
        Ok(self
            .read()?
            .filter(|(_, sha)| sha == commit_sha)
            .map(|(stored, _)| stored))
    }
}

/// The adapter conformance suite, shared by every store (INIT-013). Each check builds a fresh
/// store (and scope) through `make`.
#[cfg(feature = "test-support")]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub mod conformance {
    use super::*;

    fn sha(c: char) -> CommitSha {
        c.to_string().repeat(40).parse().unwrap()
    }

    /// Minimal valid facts for a commit and detection time.
    pub fn sample_facts(commit: char, detected_at: &str) -> RepositoryFacts {
        let mut facts = RepositoryFacts::skeleton("conformance", &sha(commit), detected_at);
        facts.facts_hash = facts.compute_hash().unwrap();
        facts
    }

    /// Runs every conformance check. Panics (test helper) on the first violation.
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    pub async fn run_conformance<S, Fut>(make: impl Fn() -> Fut)
    where
        S: RepositoryFactsStore,
        Fut: Future<Output = (S, RepoScope)>,
    {
        // save_then_latest_roundtrip
        let (store, scope) = make().await;
        let facts = sample_facts('a', "2026-01-01T00:00:00Z");
        let saved = store.save(&scope, &facts).await.unwrap();
        assert!(saved.created);
        let latest = store.latest(&scope).await.unwrap().unwrap();
        assert_eq!(latest.facts, facts);
        assert_eq!(latest.id, saved.id);

        // save_identical_twice_is_idempotent
        let (store, scope) = make().await;
        let first = store.save(&scope, &facts).await.unwrap();
        let second = store.save(&scope, &facts).await.unwrap();
        assert!(first.created && !second.created);
        assert_eq!(first.id, second.id);

        // by_commit_returns_matching
        let (store, scope) = make().await;
        store.save(&scope, &facts).await.unwrap();
        assert!(store.by_commit(&scope, &sha('a')).await.unwrap().is_some());
        assert!(store.by_commit(&scope, &sha('b')).await.unwrap().is_none());

        // newer_detected_at_wins_pointer
        let (store, scope) = make().await;
        let older = sample_facts('a', "2026-01-01T00:00:00Z");
        let newer = sample_facts('b', "2026-01-02T00:00:00Z");
        store.save(&scope, &older).await.unwrap();
        store.save(&scope, &newer).await.unwrap();
        assert_eq!(store.latest(&scope).await.unwrap().unwrap().facts, newer);

        // older_late_save_does_not_regress_pointer
        let (store, scope) = make().await;
        store.save(&scope, &newer).await.unwrap();
        store.save(&scope, &older).await.unwrap();
        assert_eq!(store.latest(&scope).await.unwrap().unwrap().facts, newer);

        // too_large_rejected
        let (store, scope) = make().await;
        let mut huge = sample_facts('c', "2026-01-03T00:00:00Z");
        huge.root_name = "x".repeat(MAX_FACTS_BYTES + 1);
        assert!(matches!(
            store.save(&scope, &huge).await,
            Err(FactsStoreError::TooLarge { .. })
        ));
        assert!(store.latest(&scope).await.unwrap().is_none());

        // unsupported_schema_version_rejected
        let (store, scope) = make().await;
        let mut future = sample_facts('d', "2026-01-04T00:00:00Z");
        future.schema_version = REPOSITORY_FACTS_SCHEMA + 1;
        assert!(matches!(
            store.save(&scope, &future).await,
            Err(FactsStoreError::SchemaUnsupported { .. })
        ));
    }
}
