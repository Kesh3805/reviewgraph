//! The `RepositoryFacts` aggregate written to `.review/repository.json` (INIT-011).
//!
//! No absolute host paths, raw remote URLs, environment values, script commands or sensitive
//! file contents ever appear here.

use std::collections::BTreeMap;

use review_core::language::Language;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::build_systems::BuildSystemFact;
use crate::docs_meta::DocsFacts;
use crate::entrypoints::EntrypointFact;
use crate::env_files::EnvFileFact;
use crate::error::{InitError, InitWarning};
use crate::frameworks::{AuthFacts, FrameworkFact};
use crate::generated::{GeneratedFacts, GeneratedKind};
use crate::git::GitState;
use crate::infra::InfraFact;
use crate::language::LanguageStat;
use crate::layout::LayoutFacts;
use crate::manifests::{ManifestFact, PackageManagerFact};
use crate::migrations::MigrationDirFact;
use crate::tooling::ToolingFacts;
use crate::tsconfig::TsConfigFact;
use crate::walk::FileInventory;
use crate::workspaces::WorkspaceLayout;

/// Bumped on any breaking change to the `repository.json` shape.
pub const REPOSITORY_FACTS_SCHEMA: u32 = 1;
/// Domain prefix of `facts_hash`.
pub const FACTS_HASH_DOMAIN: &[u8] = b"rg.facts.v1\0";

pub const MAX_DEPENDENCIES_PER_MANIFEST: usize = 2_000;
pub const MAX_ENV_VARIABLE_NAMES: usize = 500;
pub const MAX_WARNINGS: usize = 1_000;

/// Totals over the walked inventory (not the per-file list).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct InventorySummary {
    pub total_files: u64,
    pub total_dirs: u64,
    pub by_class: BTreeMap<String, u64>,
    pub ignored: BTreeMap<String, u64>,
}

impl InventorySummary {
    pub fn of(inv: &FileInventory) -> Self {
        Self {
            total_files: inv.entries.len() as u64,
            total_dirs: inv.dirs.len() as u64,
            by_class: inv
                .count_by_class()
                .into_iter()
                .map(|(k, v)| (k.as_str().to_owned(), v))
                .collect(),
            ignored: inv
                .ignored
                .iter()
                .map(|(k, v)| (format!("{k:?}").to_lowercase(), *v))
                .collect(),
        }
    }
}

/// Generated-code summary. The per-file map is kept out of `repository.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct GeneratedSummary {
    pub dirs: Vec<(RepoPath, GeneratedKind, u64)>,
    pub files: u64,
    pub by_kind: BTreeMap<String, u64>,
    pub by_reason: BTreeMap<String, u64>,
    pub config_globs: Vec<String>,
}

impl GeneratedSummary {
    pub fn of(facts: &GeneratedFacts) -> Self {
        let mut by_kind = BTreeMap::new();
        let mut by_reason = BTreeMap::new();
        for class in facts.files.values() {
            *by_kind
                .entry(format!("{:?}", class.kind).to_lowercase())
                .or_insert(0) += 1;
            *by_reason
                .entry(class.reason.label().to_owned())
                .or_insert(0) += 1;
        }
        Self {
            dirs: facts.dirs.clone(),
            files: facts.files.len() as u64,
            by_kind,
            by_reason,
            config_globs: facts.config_globs.clone(),
        }
    }
}

/// API routes need parsing, so they are produced at index time (NEST-002 / IDX).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeferredToIndex {
    pub status: String,
}

impl Default for DeferredToIndex {
    fn default() -> Self {
        Self {
            status: "deferred_to_index".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RepositoryFacts {
    pub schema_version: u32,
    pub tool_version: String,
    /// RFC 3339 UTC. Excluded from `facts_hash`.
    pub detected_at: String,
    /// Directory name only: absolute host paths are never stored.
    pub root_name: String,
    pub git: Option<GitState>,
    pub inventory: InventorySummary,
    pub languages: Vec<LanguageStat>,
    pub primary_language: Option<Language>,
    pub package_managers: Vec<PackageManagerFact>,
    pub manifests: Vec<ManifestFact>,
    pub build_systems: Vec<BuildSystemFact>,
    pub workspaces: WorkspaceLayout,
    pub frameworks: Vec<FrameworkFact>,
    pub auth: AuthFacts,
    pub layout: LayoutFacts,
    pub tsconfigs: Vec<TsConfigFact>,
    pub tooling: ToolingFacts,
    pub generated: GeneratedSummary,
    pub entrypoints: Vec<EntrypointFact>,
    pub migrations: Vec<MigrationDirFact>,
    pub schema_files: Vec<RepoPath>,
    pub infra: Vec<InfraFact>,
    pub env_files: Vec<EnvFileFact>,
    pub sensitive_files: Vec<RepoPath>,
    pub docs: DocsFacts,
    pub api_routes: DeferredToIndex,
    /// Filled by the caller through INIT-012.
    pub fingerprint: Option<String>,
    pub facts_hash: String,
    /// Sorted by (code, path), at most [`MAX_WARNINGS`].
    pub warnings: Vec<InitWarning>,
    /// Warnings dropped by the cap.
    pub warnings_truncated: u64,
}

impl RepositoryFacts {
    /// `blake3("rg.facts.v1\0" || canonical_json)` with `detected_at`, `facts_hash` and
    /// `fingerprint` blanked. Struct order and sorted maps make the JSON canonical.
    pub fn compute_hash(&self) -> Result<String, InitError> {
        let mut blank = self.clone();
        blank.detected_at = String::new();
        blank.facts_hash = String::new();
        blank.fingerprint = None;
        let json = serde_json::to_vec(&blank).map_err(|e| InitError::Serde(e.to_string()))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(FACTS_HASH_DOMAIN);
        hasher.update(&json);
        Ok(hex::encode(hasher.finalize().as_bytes()))
    }

    pub fn to_pretty_json(&self) -> Result<String, InitError> {
        let mut text =
            serde_json::to_string_pretty(self).map_err(|e| InitError::Serde(e.to_string()))?;
        text.push('\n');
        Ok(text)
    }

    /// Migrates stored facts of an older schema version. A no-op for version 1.
    pub fn upgrade_from(version: u32, json: &str) -> Result<Self, InitError> {
        if version > REPOSITORY_FACTS_SCHEMA {
            return Err(InitError::Serde(format!(
                "repository facts schema {version} is newer than {REPOSITORY_FACTS_SCHEMA}"
            )));
        }
        serde_json::from_str(json).map_err(|e| InitError::Serde(e.to_string()))
    }

    /// Applies the size caps. Every truncation is recorded as a `list_truncated` warning.
    pub fn apply_caps(&mut self) {
        for manifest in &mut self.manifests {
            if manifest.dependencies.len() > MAX_DEPENDENCIES_PER_MANIFEST {
                let keep: Vec<String> = manifest
                    .dependencies
                    .keys()
                    .take(MAX_DEPENDENCIES_PER_MANIFEST)
                    .cloned()
                    .collect();
                manifest.dependencies.retain(|k, _| keep.contains(k));
                self.warnings.push(InitWarning::new(
                    "list_truncated",
                    Some(manifest.path.clone()),
                    "dependencies were capped at 2000 entries",
                ));
            }
        }
        for env in &mut self.env_files {
            if env.variable_names.len() > MAX_ENV_VARIABLE_NAMES {
                env.variable_names.truncate(MAX_ENV_VARIABLE_NAMES);
                self.warnings.push(InitWarning::new(
                    "list_truncated",
                    Some(env.path.clone()),
                    "variable names were capped at 500 entries",
                ));
            }
        }
        self.warnings
            .sort_by(|a, b| (&a.code, &a.path, &a.message).cmp(&(&b.code, &b.path, &b.message)));
        self.warnings.dedup();
        if self.warnings.len() > MAX_WARNINGS {
            self.warnings_truncated = (self.warnings.len() - MAX_WARNINGS) as u64;
            self.warnings.truncate(MAX_WARNINGS);
        }
    }
}
