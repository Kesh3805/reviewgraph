//! Risk signals and categories (RISK-001).
//!
//! A signal says "this subject carries this kind of risk, this strongly, for this reason". It
//! names the rule that produced it, so every score is explainable down to a table row.
//! Explanations and evidence never contain code text: only paths, lines, symbol and node ids,
//! rule descriptions and, for secrets, a redacted preview of at most four characters.

use std::collections::BTreeMap;
use std::fmt;

use review_core::change::ChangeClusterKey;
use review_core::ids::SymbolKey;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::level::RiskLevel;
use super::rules::RiskRule;

/// The 18 PRD §37 risk categories.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskCategory {
    Authentication,
    Authorization,
    Cryptography,
    Permissions,
    Payments,
    DatabaseMigrations,
    SchemaChanges,
    TransactionBoundaries,
    Concurrency,
    Queues,
    BackgroundJobs,
    PublicApiContracts,
    Serialization,
    Validation,
    ExternalNetworkCalls,
    FilesystemAccess,
    Secrets,
    DependencyUpdates,
}

impl RiskCategory {
    pub const ALL: [RiskCategory; 18] = [
        Self::Authentication,
        Self::Authorization,
        Self::Cryptography,
        Self::Permissions,
        Self::Payments,
        Self::DatabaseMigrations,
        Self::SchemaChanges,
        Self::TransactionBoundaries,
        Self::Concurrency,
        Self::Queues,
        Self::BackgroundJobs,
        Self::PublicApiContracts,
        Self::Serialization,
        Self::Validation,
        Self::ExternalNetworkCalls,
        Self::FilesystemAccess,
        Self::Secrets,
        Self::DependencyUpdates,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Cryptography => "cryptography",
            Self::Permissions => "permissions",
            Self::Payments => "payments",
            Self::DatabaseMigrations => "database_migrations",
            Self::SchemaChanges => "schema_changes",
            Self::TransactionBoundaries => "transaction_boundaries",
            Self::Concurrency => "concurrency",
            Self::Queues => "queues",
            Self::BackgroundJobs => "background_jobs",
            Self::PublicApiContracts => "public_api_contracts",
            Self::Serialization => "serialization",
            Self::Validation => "validation",
            Self::ExternalNetworkCalls => "external_network_calls",
            Self::FilesystemAccess => "filesystem_access",
            Self::Secrets => "secrets",
            Self::DependencyUpdates => "dependency_updates",
        }
    }

    /// The category's base weight; a rule's weight defaults to it.
    pub const fn base_weight(self) -> f32 {
        match self {
            Self::Authorization | Self::Secrets => 0.70,
            Self::Payments => 0.65,
            Self::Authentication | Self::Cryptography | Self::Permissions => 0.60,
            Self::DatabaseMigrations => 0.55,
            Self::SchemaChanges
            | Self::TransactionBoundaries
            | Self::PublicApiContracts
            | Self::Validation => 0.50,
            Self::Concurrency => 0.45,
            Self::Queues => 0.40,
            Self::BackgroundJobs
            | Self::Serialization
            | Self::ExternalNetworkCalls
            | Self::FilesystemAccess => 0.35,
            Self::DependencyUpdates => 0.30,
        }
    }

    /// Security-relevant categories: never skipped for budget (IMP-010) and reviewed by the
    /// security reviewer (RISK-005).
    pub const fn is_security(self) -> bool {
        matches!(
            self,
            Self::Authentication
                | Self::Authorization
                | Self::Cryptography
                | Self::Permissions
                | Self::Secrets
                | Self::Validation
                | Self::Payments
        )
    }
}

impl fmt::Display for RiskCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which kind of detector a rule belongs to.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RuleSource {
    Path,
    Manifest,
    Secrets,
    Change,
    Framework,
    Graph,
}

impl RuleSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Manifest => "manifest",
            Self::Secrets => "secrets",
            Self::Change => "change",
            Self::Framework => "framework",
            Self::Graph => "graph",
        }
    }
}

/// What a signal is about.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum SignalSubject {
    Pr,
    File(RepoPath),
    Symbol(SymbolKey),
    Cluster(ChangeClusterKey),
}

/// Why a signal fired. Never holds code text or secret values.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct SignalEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<RepoPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<SymbolKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub detail: String,
    /// For secrets: at most four visible characters followed by `****`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted_preview: Option<String>,
}

impl SignalEvidence {
    pub fn detail(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            ..Self::default()
        }
    }

    pub fn at_node(node_id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            node_id: Some(node_id.into()),
            detail: detail.into(),
            ..Self::default()
        }
    }

    pub fn at_line(file: RepoPath, line: u32, detail: impl Into<String>) -> Self {
        Self {
            file: Some(file),
            line: Some(line),
            detail: detail.into(),
            ..Self::default()
        }
    }
}

/// One risk signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RiskSignal {
    /// The producing rule, e.g. `authz.call_removed`.
    pub rule_id: String,
    pub category: RiskCategory,
    pub subject: SignalSubject,
    pub source: RuleSource,
    /// In `[0, 1]`.
    pub weight: f32,
    /// In `[0, 1]`.
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor_level: Option<RiskLevel>,
    pub evidence: Vec<SignalEvidence>,
    pub explanation: String,
    /// `clamp(weight × confidence × amplifier, 0, 0.95)`, filled by scoring (RISK-004).
    #[serde(default)]
    pub contribution: f32,
}

impl RiskSignal {
    /// A signal of `rule` with the rule's weight, category, source and floor.
    pub fn from_rule(
        rule: &RiskRule,
        subject: SignalSubject,
        confidence: f32,
        evidence: Vec<SignalEvidence>,
    ) -> Self {
        Self {
            rule_id: rule.id.to_owned(),
            category: rule.category,
            subject,
            source: rule.source,
            weight: rule.weight,
            confidence: confidence.clamp(0.0, 1.0),
            floor_level: rule.floor_level,
            evidence,
            explanation: rule.description.to_owned(),
            contribution: 0.0,
        }
    }

    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight.clamp(0.0, 1.0);
        self
    }

    pub fn with_category(mut self, category: RiskCategory) -> Self {
        self.category = category;
        self
    }

    pub fn with_floor(mut self, floor: Option<RiskLevel>) -> Self {
        self.floor_level = floor;
        self
    }

    pub fn with_explanation(mut self, explanation: impl Into<String>) -> Self {
        self.explanation = explanation.into();
        self
    }
}

/// Merges signals with the same `(rule_id, subject)`: the highest confidence (and weight) wins,
/// evidence is unioned and sorted, the strongest floor is kept. Output sorted by key.
pub fn dedup_signals(signals: Vec<RiskSignal>) -> Vec<RiskSignal> {
    let mut merged: BTreeMap<(String, SignalSubject), RiskSignal> = BTreeMap::new();
    for signal in signals {
        let key = (signal.rule_id.clone(), signal.subject.clone());
        match merged.get_mut(&key) {
            None => {
                merged.insert(key, signal);
            }
            Some(existing) => {
                if signal.confidence > existing.confidence {
                    existing.confidence = signal.confidence;
                    existing.explanation = signal.explanation.clone();
                }
                existing.weight = existing.weight.max(signal.weight);
                existing.floor_level = existing.floor_level.max(signal.floor_level);
                existing.evidence.extend(signal.evidence);
            }
        }
    }
    merged
        .into_values()
        .map(|mut signal| {
            signal.evidence.sort();
            signal.evidence.dedup();
            signal
        })
        .collect()
}
