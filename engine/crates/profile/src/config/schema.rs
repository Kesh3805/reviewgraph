//! `.review/config.yaml` schema v1 (POL-001, PRD §122 + §66).
//!
//! Every struct denies unknown fields and every section has a default, so a missing section and
//! an empty one mean the same thing and the normalized form (defaults expanded) is canonical.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The only schema version this crate reads.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Lowest `confidence.minimum_publish` (and per-reviewer threshold) a repository may set.
pub const MINIMUM_PUBLISH_FLOOR: f64 = 0.55;
/// Lowest per-reviewer threshold for the maintainability reviewer.
pub const MAINTAINABILITY_FLOOR: f64 = 0.85;

/// Layer names a rule may reference without declaring them: the inferred roles (PROF-002) in
/// singular and plural form.
pub const INFERABLE_LAYER_NAMES: [&str; 22] = [
    "controller",
    "controllers",
    "service",
    "services",
    "repository",
    "repositories",
    "entity",
    "entities",
    "dto",
    "dtos",
    "guard",
    "guards",
    "processor",
    "processors",
    "module",
    "modules",
    "config",
    "configs",
    "util",
    "utils",
    "test",
    "tests",
];

/// The whole repository configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewConfigV1 {
    /// Schema version; must be `1`.
    pub version: u32,
    /// Extra ignore globs, applied on top of `.gitignore` and `.reviewignore` (INIT-002).
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Generated-code classification overrides (INIT-008).
    #[serde(default)]
    pub generated: GeneratedClassification,
    /// Indexing knobs (IDX).
    #[serde(default)]
    pub index: IndexSection,
    #[serde(default)]
    pub review: ReviewSection,
    #[serde(default)]
    pub architecture: ArchitectureSection,
    #[serde(default)]
    pub rules: RulesSection,
    #[serde(default)]
    pub conventions: ConventionsSection,
    #[serde(default)]
    pub suppressions: Vec<ConfigSuppression>,
    #[serde(default)]
    pub knowledge_sources: Vec<KnowledgeSourceConfig>,
}

impl Default for ReviewConfigV1 {
    fn default() -> Self {
        Self {
            version: CONFIG_SCHEMA_VERSION,
            ignore: Vec::new(),
            generated: GeneratedClassification::default(),
            index: IndexSection::default(),
            review: ReviewSection::default(),
            architecture: ArchitectureSection::default(),
            rules: RulesSection::default(),
            conventions: ConventionsSection::default(),
            suppressions: Vec::new(),
            knowledge_sources: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratedClassification {
    /// Force files into the generated class.
    #[serde(default)]
    pub include: Vec<String>,
    /// Force files out of the generated class.
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexSection {
    #[serde(default)]
    pub tolerance: IndexTolerance,
}

/// Parse-failure tolerance (IDX). `None` keeps the indexer's built-in default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IndexTolerance {
    #[serde(default)]
    pub max_failed_ratio: Option<f64>,
    #[serde(default)]
    pub max_failed_files: Option<u32>,
    #[serde(default)]
    pub min_parsed_files: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewSection {
    #[serde(default)]
    pub reviewers: ReviewerToggles,
    #[serde(default)]
    pub confidence: ConfidenceSection,
    #[serde(default)]
    pub budgets: Budgets,
    #[serde(default)]
    pub generated: ReviewGenerated,
    #[serde(default)]
    pub risk: RiskSection,
    #[serde(default)]
    pub privacy: PrivacySection,
    #[serde(default)]
    pub publish: PublishSection,
}

const fn yes() -> bool {
    true
}

/// Which reviewers may run. Maintainability is opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewerToggles {
    #[serde(default = "yes")]
    pub correctness: bool,
    #[serde(default = "yes")]
    pub security: bool,
    #[serde(default = "yes")]
    pub tests: bool,
    #[serde(default = "yes")]
    pub performance: bool,
    #[serde(default = "yes")]
    pub architecture: bool,
    #[serde(default)]
    pub maintainability: bool,
}

impl Default for ReviewerToggles {
    fn default() -> Self {
        Self {
            correctness: true,
            security: true,
            tests: true,
            performance: true,
            architecture: true,
            maintainability: false,
        }
    }
}

/// Reviewer names accepted as keys of `confidence.per_reviewer`.
pub const REVIEWER_NAMES: [&str; 6] = [
    "correctness",
    "security",
    "tests",
    "performance",
    "architecture",
    "maintainability",
];

const fn default_minimum_publish() -> f64 {
    0.72
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfidenceSection {
    /// Lowest computed confidence that publishes. In `[0.55, 1]`.
    #[serde(default = "default_minimum_publish")]
    pub minimum_publish: f64,
    /// Per-reviewer overrides, keyed by reviewer name.
    #[serde(default = "default_per_reviewer")]
    pub per_reviewer: BTreeMap<String, f64>,
}

fn default_per_reviewer() -> BTreeMap<String, f64> {
    BTreeMap::from([("maintainability".to_owned(), 0.90)])
}

impl Default for ConfidenceSection {
    fn default() -> Self {
        Self {
            minimum_publish: default_minimum_publish(),
            per_reviewer: default_per_reviewer(),
        }
    }
}

const fn default_max_symbols() -> u32 {
    100
}
const fn default_max_context_tokens() -> u32 {
    40_000
}
const fn default_max_model_calls() -> u32 {
    30
}
const fn default_max_review_seconds() -> u32 {
    300
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    #[serde(default = "default_max_symbols")]
    pub max_symbols: u32,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: u32,
    #[serde(default = "default_max_model_calls")]
    pub max_model_calls: u32,
    #[serde(default = "default_max_review_seconds")]
    pub max_review_seconds: u32,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_symbols: default_max_symbols(),
            max_context_tokens: default_max_context_tokens(),
            max_model_calls: default_max_model_calls(),
            max_review_seconds: default_max_review_seconds(),
        }
    }
}

/// Globs whose files are never reviewed (they are still indexed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewGenerated {
    #[serde(default)]
    pub ignore: Vec<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RiskSection {
    /// Glob → risk level.
    #[serde(default)]
    pub paths: BTreeMap<String, RiskLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrivacySection {
    /// `false` keeps every model call on eligible local providers (ADR-010).
    #[serde(default = "yes")]
    pub external_models: bool,
}

impl Default for PrivacySection {
    fn default() -> Self {
        Self {
            external_models: true,
        }
    }
}

const fn default_inline_cap() -> u32 {
    25
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishSection {
    #[serde(default = "default_inline_cap")]
    pub inline_cap: u32,
    #[serde(default = "yes")]
    pub summary: bool,
    #[serde(default = "yes")]
    pub check_run: bool,
}

impl Default for PublishSection {
    fn default() -> Self {
        Self {
            inline_cap: default_inline_cap(),
            summary: true,
            check_run: true,
        }
    }
}

/// Declared layers: layer name → file globs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureSection {
    #[serde(default)]
    pub layers: BTreeMap<String, Vec<String>>,
}

/// Severity attached to an explicit rule.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RuleSeverity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl RuleSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub const fn to_core(self) -> review_core::finding::Severity {
        use review_core::finding::Severity;
        match self {
            Self::Info => Severity::Info,
            Self::Low => Severity::Low,
            Self::Medium => Severity::Medium,
            Self::High => Severity::High,
            Self::Critical => Severity::Critical,
        }
    }
}

const fn default_rule_severity() -> RuleSeverity {
    RuleSeverity::High
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulesSection {
    #[serde(default)]
    pub forbidden_dependencies: Vec<ForbiddenDependency>,
    #[serde(default)]
    pub queue_jobs: QueueJobsRule,
    #[serde(default)]
    pub database: DatabaseRule,
    #[serde(default)]
    pub tests: TestsRule,
    #[serde(default)]
    pub security: SecurityRule,
}

/// `from` layer must not depend on `to` layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenDependency {
    pub id: String,
    pub from: String,
    pub to: String,
    #[serde(default = "default_rule_severity")]
    pub severity: RuleSeverity,
    pub reason: String,
    /// The corrective direction rendered into the comment.
    #[serde(default)]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueJobsRule {
    #[serde(default)]
    pub require_deterministic_id: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatabaseRule {
    #[serde(default)]
    pub migrations_only: bool,
    #[serde(default)]
    pub migration_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestsRule {
    #[serde(default)]
    pub public_api_changes_require_tests: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityRule {
    /// Symbols that perform authorization (`Class.method`).
    #[serde(default)]
    pub authorization_symbols: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConventionsSection {
    /// Globs excluded from every inferred convention.
    #[serde(default)]
    pub exceptions: Vec<String>,
}

/// What a suppression matches on (POL-006).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SuppressionKind {
    /// A reviewer category such as `security.input_validation`.
    Type,
    /// A glob on the anchor path.
    Path,
    /// A symbol id prefix.
    Symbol,
    /// A rule id.
    Rule,
    /// A root-cause fingerprint.
    Fingerprint,
}

impl SuppressionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Path => "path",
            Self::Symbol => "symbol",
            Self::Rule => "rule",
            Self::Fingerprint => "fingerprint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigSuppression {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: SuppressionKind,
    pub value: String,
    pub reason: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub expires: Option<NaiveDate>,
    /// `type` and `path` suppressions never match critical findings unless this is set.
    #[serde(default)]
    pub allow_critical: bool,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceKind {
    MarkdownVault,
    Adr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeSourceConfig {
    pub id: String,
    pub kind: KnowledgeSourceKind,
    /// Repository-relative directory.
    pub path: String,
}
