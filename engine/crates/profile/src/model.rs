//! `RepositoryProfile`: the typed, versioned profile of one repository snapshot (PROF-001,
//! PRD §62).
//!
//! A profile holds code facts only, never source bodies. It is immutable per
//! `(repository_id, snapshot_id, profile_version, config_hash)`.

use chrono::{DateTime, Utc};
use review_core::ids::{RepositoryId, SnapshotId};
use review_core::version::{ConfigHash, ProfileVersion};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The natural key of a stored profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileKey {
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub profile_version: ProfileVersion,
    pub config_hash: ConfigHash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryProfile {
    pub repository_id: RepositoryId,
    pub snapshot_id: SnapshotId,
    pub profile_version: ProfileVersion,
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
    /// Inference notes such as `architecture_inference: insufficient_signal`.
    pub notes: Vec<ProfileNote>,
    pub computed_at: DateTime<Utc>,
}

impl RepositoryProfile {
    /// An empty profile for a key: what inference starts from.
    pub fn empty(key: ProfileKey, computed_at: DateTime<Utc>) -> Self {
        Self {
            repository_id: key.repository_id,
            snapshot_id: key.snapshot_id,
            profile_version: key.profile_version,
            config_hash: key.config_hash,
            languages: Vec::new(),
            frameworks: Vec::new(),
            architecture: ArchitectureModel::default(),
            modules: Vec::new(),
            conventions: Vec::new(),
            testing: TestingProfile::default(),
            api: ApiProfile::default(),
            security: SecurityProfile::default(),
            persistence: PersistenceProfile::default(),
            queues: QueueProfile::default(),
            naming: NamingProfile::default(),
            errors: ErrorHandlingProfile::default(),
            documentation_rules: Vec::new(),
            historical: None,
            notes: Vec::new(),
            computed_at,
        }
    }

    pub fn key(&self) -> ProfileKey {
        ProfileKey {
            repository_id: self.repository_id,
            snapshot_id: self.snapshot_id,
            profile_version: self.profile_version,
            config_hash: self.config_hash,
        }
    }

    pub fn convention(&self, id: &str) -> Option<&Convention> {
        self.conventions.iter().find(|c| c.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanguageShare {
    pub language: String,
    pub files: u32,
    /// Fraction of source files, in `[0, 1]`.
    pub share: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameworkFact {
    pub id: String,
    pub version: Option<String>,
    pub confidence: f32,
}

/// Inferred layers and the observed dependencies between them (PROF-002).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureModel {
    pub layers: Vec<Layer>,
    /// Observed layer → layer dependency counts, sorted by `(from, to)`.
    pub matrix: Vec<LayerDependency>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// A role name (`controller`, `service`, ...).
    pub name: String,
    /// Globs describing the members (suffix and directory patterns, capped).
    pub globs: Vec<String>,
    pub member_count: u32,
    /// Mean confidence of the members' role classification.
    pub role_confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerDependency {
    pub from: String,
    pub to: String,
    pub imports: u32,
    pub calls: u32,
    pub depends_on: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModuleKind {
    /// A framework module declaration (for example a NestJS `@Module`).
    FrameworkModule,
    WorkspacePackage,
    /// A top-level directory under a source root.
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModuleBoundary {
    pub id: String,
    pub kind: ModuleKind,
    /// Repository-relative root directory.
    pub root: String,
    /// Exported names (framework `exports` or a barrel `index.ts`).
    pub public_exports: Vec<String>,
    /// Number of files outside the module that depend on it.
    pub internal_dependents: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConventionSource {
    Inferred,
    Documented,
    Explicit,
}

impl ConventionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inferred => "inferred",
            Self::Documented => "documented",
            Self::Explicit => "explicit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionReason {
    /// `// review-ignore: <convention-id>` on or above the sample.
    Annotation,
    /// Under a generated-code glob.
    Generated,
    /// Listed in config `conventions.exceptions`.
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConventionException {
    pub path: String,
    pub symbol: Option<String>,
    pub reason: ExceptionReason,
}

/// A located sample, used for violations kept as evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConventionEvidence {
    pub path: String,
    pub line: u32,
    pub symbol: Option<String>,
    pub detail: String,
}

/// An inferred (or documented, or explicit) convention (PRD §63, target-architecture §3.10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Convention {
    pub id: String,
    /// The convention as a sentence.
    pub rule: String,
    /// The glob the statistics are computed over.
    pub scope: String,
    pub samples: u32,
    pub violations: u32,
    /// `conforming / (conforming + violating)`.
    pub consistency: f32,
    /// `consistency × sample_factor × spread_factor` (PROF-003).
    pub confidence: f32,
    /// Confidence ≥ 0.9 and samples ≥ 10. Everything else is informational.
    pub enforceable: bool,
    pub exceptions: Vec<ConventionException>,
    /// Up to 20 violating samples with their evidence.
    pub violation_examples: Vec<ConventionEvidence>,
    pub source: ConventionSource,
    pub miner_version: u32,
    pub computed_at: DateTime<Utc>,
    pub profile_version: ProfileVersion,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestLayout {
    Colocated,
    Separate,
    Mixed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MockStyle {
    JestMock,
    ProviderOverride,
    Mixed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestingProfile {
    pub frameworks: Vec<String>,
    /// Test file patterns seen (`**/*.spec.ts`, ...), most frequent first.
    pub file_patterns: Vec<String>,
    pub layout: TestLayout,
    /// `colocated / (colocated + separate)` test files.
    pub colocated_ratio: f32,
    pub mock_style: MockStyle,
    pub e2e_roots: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiVersioning {
    Path,
    Header,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DtoValidation {
    Global,
    PerRoute,
    #[default]
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiProfile {
    pub endpoints: u32,
    pub versioning: ApiVersioning,
    /// Share of endpoints following the majority versioning scheme.
    pub versioning_consistency: f32,
    pub dto_validation: DtoValidation,
    pub response_envelope: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityProfile {
    pub guarded_endpoints: u32,
    pub unguarded_endpoints: u32,
    pub global_guards: Vec<String>,
    /// Symbols most frequently called first in guarded handlers (PROF-004).
    pub authorization_symbols: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PersistenceProfile {
    pub entities: u32,
    pub tables: u32,
    pub migration_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueInfo {
    /// `queue:{name}`.
    pub id: String,
    pub name: String,
    pub producers: Vec<String>,
    pub consumers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueProfile {
    pub queues: Vec<QueueInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuffixCount {
    pub suffix: String,
    pub files: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamingProfile {
    /// Role suffixes (`controller`, `service`, ...) and how many files carry them.
    pub file_suffixes: Vec<SuffixCount>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorHandlingProfile {
    /// The discovered domain error base class (PROF-004), if any.
    pub base_error_class: Option<String>,
    pub thrown_classes: u32,
}

/// A documentation rule known to the profile (PROF-007). The body is never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentationRuleRef {
    /// `doc:{source_id}/{rule_id}`.
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub kind: String,
    pub severity: Option<String>,
    pub path: String,
    pub applies_to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoricalSignalsRef {
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileNote {
    pub code: String,
    pub message: String,
}

/// The JSON Schema of [`RepositoryProfile`] (published in contracts as `RepositoryProfile`).
pub fn json_schema() -> schemars::schema::RootSchema {
    schemars::gen::SchemaGenerator::default().into_root_schema_for::<RepositoryProfile>()
}
