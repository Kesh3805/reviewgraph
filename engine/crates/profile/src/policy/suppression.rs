//! Suppression matching (POL-006, PRD §124).
//!
//! Two sources feed one [`SuppressionSet`]: config `suppressions:` (from the **base** policy, and
//! none at all when the config is invalid) and API-created suppressions (the "intentional"
//! feedback action). The set is read once at the start of verification, so mid-run changes apply
//! to the next run.
//!
//! Matching semantics:
//! * `type` — the finding type (`security.input_validation`): exact, its category, or a
//!   `prefix.*` pattern.
//! * `path` — a glob on the anchor path.
//! * `symbol` — a symbol id prefix, also matched against the symbol's earlier ids (lineage), so a
//!   suppression follows a rename.
//! * `rule` — a rule id.
//! * `fingerprint` — the root-cause fingerprint.
//!
//! Guardrails: every suppression has a reason; expired or revoked ones never match (the doctor
//! lists expired ones); `type` and `path` suppressions never match a critical finding unless
//! `allow_critical` is set.

use chrono::{DateTime, NaiveTime, Utc};
use globset::Glob;
use review_core::finding::Severity;
use serde::{Deserialize, Serialize};

use crate::config::{LoadedConfig, SuppressionKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppressionSource {
    Config,
    Api,
}

impl SuppressionSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Api => "api",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveSuppression {
    /// The config id (`s1`) or the API row id.
    pub id: String,
    pub kind: SuppressionKind,
    pub value: String,
    pub source: SuppressionSource,
    pub reason: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
    pub allow_critical: bool,
}

impl ActiveSuppression {
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        !self.revoked && self.expires_at.is_none_or(|e| e > now) && !self.reason.trim().is_empty()
    }
}

/// What matching needs to know about a candidate finding.
#[derive(Debug, Clone, Copy)]
pub struct FindingFacts<'a> {
    /// `category` or `category.subtype` (`security.input_validation`).
    pub finding_type: &'a str,
    pub path: &'a str,
    pub symbols: &'a [String],
    pub rule_id: Option<&'a str>,
    pub fingerprint: Option<&'a str>,
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuppressionMatch {
    pub suppression_id: String,
    pub source: SuppressionSource,
    pub kind: SuppressionKind,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct SuppressionSet {
    items: Vec<ActiveSuppression>,
}

fn type_matches(pattern: &str, finding_type: &str) -> bool {
    if pattern == finding_type {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return finding_type == prefix || finding_type.starts_with(&format!("{prefix}."));
    }
    // A bare category suppresses every subtype of it.
    !pattern.contains('.') && finding_type.starts_with(&format!("{pattern}."))
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    Glob::new(pattern).is_ok_and(|g| g.compile_matcher().is_match(path))
}

impl SuppressionSet {
    pub fn new(items: Vec<ActiveSuppression>) -> Self {
        Self { items }
    }

    /// The config suppressions of the (base) policy. Empty for an invalid config.
    pub fn from_config(loaded: &LoadedConfig) -> Self {
        let items = loaded
            .applicable_suppressions()
            .iter()
            .map(|s| ActiveSuppression {
                id: s.id.clone(),
                kind: s.kind,
                value: s.value.clone(),
                source: SuppressionSource::Config,
                reason: s.reason.clone(),
                // `expires` is the first day the suppression no longer applies.
                expires_at: s.expires.map(|d| d.and_time(NaiveTime::MIN).and_utc()),
                revoked: false,
                allow_critical: s.allow_critical,
            })
            .collect();
        Self { items }
    }

    /// Adds API-created suppressions.
    pub fn with_api(mut self, api: impl IntoIterator<Item = ActiveSuppression>) -> Self {
        self.items.extend(api);
        self
    }

    pub fn items(&self) -> &[ActiveSuppression] {
        &self.items
    }

    /// Suppressions past their expiry, for `review doctor`.
    pub fn expired(&self, now: DateTime<Utc>) -> Vec<&ActiveSuppression> {
        self.items
            .iter()
            .filter(|s| !s.revoked && s.expires_at.is_some_and(|e| e <= now))
            .collect()
    }

    /// The first live suppression matching the finding. `lineage` returns the earlier ids of a
    /// symbol (ADR-005), so a symbol suppression follows renames.
    pub fn match_finding(
        &self,
        finding: &FindingFacts<'_>,
        now: DateTime<Utc>,
        lineage: &dyn Fn(&str) -> Vec<String>,
    ) -> Option<SuppressionMatch> {
        let hit = self.items.iter().filter(|s| s.is_live(now)).find(|s| {
            if matches!(s.kind, SuppressionKind::Type | SuppressionKind::Path)
                && finding.severity == Severity::Critical
                && !s.allow_critical
            {
                return false;
            }
            match s.kind {
                SuppressionKind::Type => type_matches(&s.value, finding.finding_type),
                SuppressionKind::Path => glob_matches(&s.value, finding.path),
                SuppressionKind::Rule => finding.rule_id == Some(s.value.as_str()),
                SuppressionKind::Fingerprint => finding.fingerprint == Some(s.value.as_str()),
                SuppressionKind::Symbol => finding.symbols.iter().any(|symbol| {
                    symbol.starts_with(&s.value)
                        || lineage(symbol).iter().any(|old| old.starts_with(&s.value))
                }),
            }
        })?;
        crate::metrics::suppressed_finding(hit.kind.as_str());
        Some(SuppressionMatch {
            suppression_id: hit.id.clone(),
            source: hit.source,
            kind: hit.kind,
            reason: hit.reason.clone(),
        })
    }
}
