//! `Repository`, `RepositorySnapshot` and `SourceFile` entities (DOM-004).
//!
//! These map onto `repositories` (DOM-009) and `snapshots` / `file_versions` (GS-002).
//! A `Repository` carries no clone URL with credentials and no tokens.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::ids::{CommitSha, FileVersionId, OrganizationId, RepositoryId, SnapshotId};
use crate::language::Language;
use crate::location::{ContentHash, RepoPath};
use crate::provenance::Provenance;
use crate::version::AnalyzerVersion;

/// Source-control provider. Non-exhaustive so GitLab and Bitbucket can be added later (MP-*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ProviderKind {
    Github,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub id: RepositoryId,
    pub organization_id: OrganizationId,
    pub provider: ProviderKind,
    pub provider_repo_id: String,
    pub full_name: String,
    pub default_branch: String,
    pub visibility: Visibility,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full snapshots stand alone; a delta names its base, so a delta without a base cannot be
/// represented (ADR-003). Wire form: `{"kind":"full"}` or `{"kind":"delta","base":"<id>"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapshotKind {
    Full,
    Delta { base: SnapshotId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    Building,
    Ready,
    Failed,
    /// The consistency validator found a mismatch (ADR-004).
    Inconsistent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotStats {
    pub files: u64,
    pub symbols: u64,
    pub edges: u64,
    pub unresolved_refs: u64,
    pub parse_errors: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositorySnapshot {
    pub id: SnapshotId,
    pub repository_id: RepositoryId,
    pub commit_sha: CommitSha,
    pub kind: SnapshotKind,
    pub status: SnapshotStatus,
    pub provenance: Provenance,
    pub stats: SnapshotStats,
    pub created_at: DateTime<Utc>,
}

impl RepositorySnapshot {
    /// A new full snapshot, in the `Building` state.
    pub fn new_full(
        id: SnapshotId,
        repository_id: RepositoryId,
        commit_sha: CommitSha,
        provenance: Provenance,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            repository_id,
            commit_sha,
            kind: SnapshotKind::Full,
            status: SnapshotStatus::Building,
            provenance,
            stats: SnapshotStats::default(),
            created_at,
        }
    }

    /// A new delta snapshot on top of `base`, in the `Building` state. The base must belong to
    /// the same repository and be `Ready`.
    pub fn new_delta(
        base: &RepositorySnapshot,
        id: SnapshotId,
        repository_id: RepositoryId,
        commit_sha: CommitSha,
        provenance: Provenance,
        created_at: DateTime<Utc>,
    ) -> Result<Self, CoreError> {
        if base.repository_id != repository_id {
            return Err(CoreError::InvalidId {
                kind: "SnapshotBase",
                reason: format!(
                    "base snapshot {} belongs to repository {}, not {}",
                    base.id, base.repository_id, repository_id
                ),
            });
        }
        if base.status != SnapshotStatus::Ready {
            return Err(CoreError::InvalidId {
                kind: "SnapshotBase",
                reason: format!("base snapshot {} is not ready", base.id),
            });
        }
        Ok(Self {
            id,
            repository_id,
            commit_sha,
            kind: SnapshotKind::Delta { base: base.id },
            status: SnapshotStatus::Building,
            provenance,
            stats: SnapshotStats::default(),
            created_at,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Generated,
    Binary,
    TooLarge,
    UnsupportedLanguage,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ParseStatus {
    Parsed,
    ParsedWithErrors,
    Skipped { reason: SkipReason },
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
    pub file_version_id: FileVersionId,
    pub repository_id: RepositoryId,
    pub path: RepoPath,
    pub content_hash: ContentHash,
    pub language: Option<Language>,
    pub size_bytes: u64,
    pub analyzer_version: Option<AnalyzerVersion>,
    pub parse_status: ParseStatus,
    pub is_generated: bool,
}

/// Identity of a file version for caching: `(repository_id, path, content_hash,
/// analyzer_version)` (ADR-003).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceFileCacheKey {
    pub repository_id: RepositoryId,
    pub path: RepoPath,
    pub content_hash: ContentHash,
    pub analyzer_version: Option<AnalyzerVersion>,
}

impl SourceFile {
    pub fn cache_key(&self) -> SourceFileCacheKey {
        SourceFileCacheKey {
            repository_id: self.repository_id,
            path: self.path.clone(),
            content_hash: self.content_hash,
            analyzer_version: self.analyzer_version.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::{ConfigHash, GraphSchemaVersion, ProfileVersion};
    use chrono::TimeZone;
    use std::collections::BTreeMap;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn provenance() -> Provenance {
        Provenance {
            commit_sha: "a".repeat(40).parse().unwrap(),
            graph_schema_version: GraphSchemaVersion(1),
            analyzer_versions: BTreeMap::new(),
            config_hash: ConfigHash::of(b"c"),
            profile_version: ProfileVersion(1),
            model: None,
        }
    }

    fn full(repo: RepositoryId, status: SnapshotStatus) -> RepositorySnapshot {
        let mut s = RepositorySnapshot::new_full(
            SnapshotId::new(),
            repo,
            "a".repeat(40).parse().unwrap(),
            provenance(),
            now(),
        );
        s.status = status;
        s
    }

    #[test]
    fn snapshot_kind_serde_tagged_form() {
        assert_eq!(
            serde_json::to_string(&SnapshotKind::Full).unwrap(),
            r#"{"kind":"full"}"#
        );
        let base = SnapshotId::new();
        let json = serde_json::to_string(&SnapshotKind::Delta { base }).unwrap();
        assert_eq!(json, format!(r#"{{"kind":"delta","base":"{base}"}}"#));
        assert_eq!(
            serde_json::from_str::<SnapshotKind>(&json).unwrap(),
            SnapshotKind::Delta { base }
        );
        assert!(serde_json::from_str::<SnapshotKind>(r#"{"kind":"delta"}"#).is_err());
    }

    #[test]
    fn delta_requires_ready_base_same_repo() {
        let repo = RepositoryId::new();
        let ready = full(repo, SnapshotStatus::Ready);
        let delta = RepositorySnapshot::new_delta(
            &ready,
            SnapshotId::new(),
            repo,
            "b".repeat(40).parse().unwrap(),
            provenance(),
            now(),
        )
        .unwrap();
        assert_eq!(delta.kind, SnapshotKind::Delta { base: ready.id });
        assert_eq!(delta.status, SnapshotStatus::Building);

        let other_repo = RepositoryId::new();
        let wrong_repo = RepositorySnapshot::new_delta(
            &ready,
            SnapshotId::new(),
            other_repo,
            "b".repeat(40).parse().unwrap(),
            provenance(),
            now(),
        );
        assert!(matches!(
            wrong_repo,
            Err(CoreError::InvalidId {
                kind: "SnapshotBase",
                ..
            })
        ));

        for status in [
            SnapshotStatus::Building,
            SnapshotStatus::Failed,
            SnapshotStatus::Inconsistent,
        ] {
            let base = full(repo, status);
            let r = RepositorySnapshot::new_delta(
                &base,
                SnapshotId::new(),
                repo,
                "b".repeat(40).parse().unwrap(),
                provenance(),
                now(),
            );
            assert!(r.is_err(), "{status:?}");
        }
    }

    #[test]
    fn repository_roundtrips_and_rejects_unknown_fields() {
        let repo = Repository {
            id: RepositoryId::new(),
            organization_id: OrganizationId::new(),
            provider: ProviderKind::Github,
            provider_repo_id: "42".into(),
            full_name: "acme/app".into(),
            default_branch: "main".into(),
            visibility: Visibility::Private,
            archived: false,
            created_at: now(),
            updated_at: now(),
        };
        let json = serde_json::to_string(&repo).unwrap();
        assert!(json.contains("\"provider\":\"github\""));
        assert_eq!(serde_json::from_str::<Repository>(&json).unwrap(), repo);
        let extra = json.replacen('{', r#"{"token":"x","#, 1);
        assert!(serde_json::from_str::<Repository>(&extra).is_err());
    }

    #[test]
    fn source_file_cache_key_ignores_ids_and_status() {
        let repo = RepositoryId::new();
        let mk = |status| SourceFile {
            file_version_id: FileVersionId::new(),
            repository_id: repo,
            path: RepoPath::new("src/a.ts").unwrap(),
            content_hash: ContentHash::of(b"x"),
            language: Some(Language::Typescript),
            size_bytes: 1,
            analyzer_version: Some(AnalyzerVersion::new(1, 0, 0)),
            parse_status: status,
            is_generated: false,
        };
        let a = mk(ParseStatus::Parsed);
        let b = mk(ParseStatus::Failed);
        assert_eq!(a.cache_key(), b.cache_key());
        let json = serde_json::to_string(&mk(ParseStatus::Skipped {
            reason: SkipReason::TooLarge,
        }))
        .unwrap();
        assert!(json.contains(r#""parse_status":{"status":"skipped","reason":"too_large"}"#));
    }
}
