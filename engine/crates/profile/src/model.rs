//! The `RepositoryProfile` model (PROF-001, PRD §62): typed facts about how a repository is
//! built, keyed to one snapshot and one config hash. Code facts only; never source bodies.

use chrono::{DateTime, Utc};
use review_core::ids::{RepositoryId, SnapshotId};
use review_core::version::ConfigHash;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bumped when the shape or the inference semantics of the profile change.
pub const PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LanguageShare {
    pub language: String,
    pub files: u32,
    pub share: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FrameworkFact {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LayerDef {
    pub name: String,
    pub globs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchitectureModel {
    pub layers: Vec<LayerDef>,
    /// Allowed layer dependencies as `(from, to)` pairs.
    pub allowed_layer_edges: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleBoundary {
    pub name: String,
    pub root: String,
    pub layer: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConventionSource {
    Inferred,
    Documented,
    Explicit,
}

impl ConventionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inferred => "inferred",
            Self::Documented => "documented",
            Self::Explicit => "explicit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConventionException {
    pub path: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Convention {
    pub id: String,
    pub rule: String,
    /// Glob the convention applies to.
    pub scope: String,
    pub samples: u32,
    pub violations: u32,
    pub consistency: f32,
    pub confidence: f32,
    #[serde(default)]
    pub exceptions: Vec<ConventionException>,
    pub source: ConventionSource,
    pub computed_at: DateTime<Utc>,
    pub profile_version: u32,
}

macro_rules! profile_section {
    ($(#[$m:meta])* $name:ident { $($f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        pub struct $name { $(#[serde(default)] pub $f: $t),* }
    };
}

profile_section!(
    /// Test layout and tooling.
    TestingProfile { frameworks: Vec<String>, test_globs: Vec<String>, colocated: Option<bool> }
);
profile_section!(
    /// HTTP/API surface conventions.
    ApiProfile { style: Option<String>, versioned: Option<bool>, dto_validation: Option<bool> }
);
profile_section!(
    /// Authentication and authorization patterns.
    SecurityProfile { guard_symbols: Vec<String>, authorization_symbols: Vec<String> }
);
profile_section!(
    /// Persistence layer facts.
    PersistenceProfile { orm: Option<String>, transaction_wrappers: Vec<String> }
);
profile_section!(
    /// Queue and background-job facts.
    QueueProfile { libraries: Vec<String>, idempotency_keys: bool }
);
profile_section!(
    /// Naming conventions.
    NamingProfile { file_suffixes: Vec<String>, case_style: Option<String> }
);
profile_section!(
    /// Error handling conventions.
    ErrorHandlingProfile { base_errors: Vec<String>, wrapper_symbols: Vec<String> }
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DocumentationRuleRef {
    pub rule_id: String,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HistoricalSignalsRef {
    pub snapshot_id: SnapshotId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RepositoryProfile {
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub profile_version: u32,
    pub config_hash: ConfigHash,
    pub languages: Vec<LanguageShare>,
    pub frameworks: Vec<FrameworkFact>,
    pub architecture: ArchitectureModel,
    pub modules: Vec<ModuleBoundary>,
    pub conventions: Vec<Convention>,
    pub testing: TestingProfile,
    pub api: ApiProfile,
    pub security: SecurityProfile,
    pub persistence: PersistenceProfile,
    pub queues: QueueProfile,
    pub naming: NamingProfile,
    pub errors: ErrorHandlingProfile,
    pub documentation_rules: Vec<DocumentationRuleRef>,
    pub historical: Option<HistoricalSignalsRef>,
    pub computed_at: DateTime<Utc>,
}

impl RepositoryProfile {
    /// An empty profile for a snapshot (the shape used before inference runs).
    pub fn empty(
        repository_id: RepositoryId,
        snapshot_id: SnapshotId,
        config_hash: ConfigHash,
        computed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            repository_id,
            snapshot_id,
            profile_version: PROFILE_VERSION,
            config_hash,
            languages: vec![],
            frameworks: vec![],
            architecture: ArchitectureModel::default(),
            modules: vec![],
            conventions: vec![],
            testing: TestingProfile::default(),
            api: ApiProfile::default(),
            security: SecurityProfile::default(),
            persistence: PersistenceProfile::default(),
            queues: QueueProfile::default(),
            naming: NamingProfile::default(),
            errors: ErrorHandlingProfile::default(),
            documentation_rules: vec![],
            historical: None,
            computed_at,
        }
    }

    /// The natural key of a stored profile.
    pub fn key(&self) -> ProfileKey {
        ProfileKey {
            repository_id: self.repository_id,
            snapshot_id: self.snapshot_id,
            profile_version: self.profile_version,
            config_hash: self.config_hash,
        }
    }
}

/// `(repository_id, snapshot_id, profile_version, config_hash)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileKey {
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub profile_version: u32,
    pub config_hash: ConfigHash,
}

/// The generated JSON Schema of [`RepositoryProfile`] (exported to `packages/contracts`).
pub fn json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(RepositoryProfile)
}
