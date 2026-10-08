//! Reviewer routing (REV-002): which reviewers run on which cluster, with which focus.
//!
//! [`plan_reviewers`] is a pure function. The rule table (PRD §48) is evaluated per cluster and
//! the union of matching rows is enabled:
//!
//! | Condition (cluster signals) | correctness | security | tests | architecture | performance | maintainability |
//! |---|---|---|---|---|---|---|
//! | only docs/comments/formatting (low risk) | – | – | – | – | – | – |
//! | behavioural symbol change | ✓ | | ✓ | | | |
//! | auth/authorization/guard/permission | ✓ | ✓ | ✓ | ✓ | | |
//! | database write / migration / transaction boundary | ✓ + `database_safety` | | ✓ | ✓ | | |
//! | API contract changed | ✓ | ✓ if public endpoint | ✓ | | | |
//! | loop/query/IO added in a hot path | | | | | ✓ | |
//! | dependency manifest changed | | ✓ | | ✓ | | |
//! | large complexity growth | | | | | | ✓ (only if enabled in config) |
//!
//! **Stand-in inputs.** CHG/RISK/IMP do not exist yet, so a cluster carries its change and risk
//! signals directly ([`ChangeCluster::signals`]); the signal names are the ones listed in the
//! constants below.

use std::collections::{BTreeMap, BTreeSet};

use globset::{Glob, GlobSet, GlobSetBuilder};
use model_gateway::{ModelTier, RiskBand};
use serde::{Deserialize, Serialize};

use crate::focus::{active_profiles, FocusProfile};
use crate::reviewer::{ContextBudget, ReviewerKind, RiskAssessment};

/// Change classes that do not change behaviour.
pub const NON_BEHAVIOURAL: &[&str] = &[
    "formatting",
    "whitespace",
    "comment_only",
    "docs_only",
    "rename_pure",
];
pub const AUTH_SIGNALS: &[&str] = &[
    "authorization_logic_changed",
    "authentication_changed",
    "guard_changed",
    "permission_changed",
    "auth_path_high_risk",
];
pub const DATABASE_SIGNALS: &[&str] = &[
    "database_write_changed",
    "migration_added",
    "transaction_boundary_changed",
];
pub const API_SIGNALS: &[&str] = &[
    "api_contract_changed",
    "route_changed",
    "dto_changed",
    "exported_signature_changed",
];
pub const PERFORMANCE_SIGNALS: &[&str] =
    &["loop_added_hot_path", "query_in_loop", "io_in_hot_path"];
pub const DEPENDENCY_SIGNALS: &[&str] = &["dependency_manifest_changed"];
pub const COMPLEXITY_SIGNALS: &[&str] = &["complexity_growth"];

/// Reviewer precedence used for ordering (and budget allocation, PIPE-006).
pub const PRECEDENCE: [ReviewerKind; 6] = [
    ReviewerKind::Security,
    ReviewerKind::Correctness,
    ReviewerKind::Test,
    ReviewerKind::Architecture,
    ReviewerKind::Performance,
    ReviewerKind::Maintainability,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterSymbol {
    pub symbol_id: String,
    pub path: String,
    /// Detected as generated or vendored at index time (INIT-008).
    #[serde(default)]
    pub generated: bool,
}

/// A cluster of related changes (IMP-009 stand-in).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeCluster {
    pub cluster_id: String,
    /// Cluster risk score; higher first.
    pub risk_score: f64,
    pub signals: BTreeSet<String>,
    pub symbols: Vec<ClusterSymbol>,
    /// The cluster reaches a public HTTP endpoint.
    #[serde(default)]
    pub public_endpoint: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    NotApplicable,
    DisabledByConfig,
    NotImplemented,
    GeneratedOnly,
    LowRiskOnly,
    BudgetExhausted,
}

/// Where the reviewer configuration was read from. A PR cannot change its own review: only the
/// base revision's `.review/config.yaml` may disable reviewers (POL-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    #[default]
    Base,
    Head,
}

/// `review.reviewers.<kind>` toggles and `generated.ignore` globs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewersConfig {
    /// `false` disables a reviewer; `true` makes it eligible (not forced).
    #[serde(default)]
    pub toggles: BTreeMap<ReviewerKind, bool>,
    #[serde(default)]
    pub generated_ignore: Vec<String>,
    #[serde(default)]
    pub source: ConfigSource,
}

/// Which reviewers have an implementation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewerRegistry {
    implemented: BTreeSet<ReviewerKind>,
}

impl ReviewerRegistry {
    pub fn new(implemented: impl IntoIterator<Item = ReviewerKind>) -> Self {
        Self {
            implemented: implemented.into_iter().collect(),
        }
    }

    /// The reviewers implemented today.
    pub fn current() -> Self {
        Self::new([ReviewerKind::Correctness])
    }

    pub fn is_implemented(&self, kind: ReviewerKind) -> bool {
        self.implemented.contains(&kind)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub reviewer: ReviewerKind,
    pub cluster_id: String,
    pub focus: Vec<FocusProfile>,
    pub budget: ContextBudget,
    pub tier: ModelTier,
    pub reasons: Vec<String>,
    /// Symbols sent to the reviewer (generated ones removed).
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub reviewer: Option<ReviewerKind>,
    pub cluster_id: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewerPlan {
    pub entries: Vec<PlanEntry>,
    pub skipped: Vec<SkippedEntry>,
}

fn any(signals: &BTreeSet<String>, set: &[&str]) -> bool {
    set.iter().any(|s| signals.contains(*s))
}

fn generated_globs(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Tier for a reviewer: `deep_reasoner` only for a critical, multi-module change.
pub fn tier_for(kind: ReviewerKind, risk: Option<&RiskAssessment>) -> ModelTier {
    match risk {
        Some(r)
            if r.level == RiskBand::Critical
                && r.modules_touched >= 2
                && matches!(kind, ReviewerKind::Correctness | ReviewerKind::Security) =>
        {
            ModelTier::DeepReasoner
        }
        _ => ModelTier::ReviewReasoner,
    }
}

/// Reviewers the rule table enables for one cluster, with reasons.
fn rule_table(
    cluster: &ChangeCluster,
    risk_known: bool,
) -> BTreeMap<ReviewerKind, BTreeSet<&'static str>> {
    let s = &cluster.signals;
    let mut on: BTreeMap<ReviewerKind, BTreeSet<&'static str>> = BTreeMap::new();
    let mut enable = |k: ReviewerKind, why: &'static str| {
        on.entry(k).or_default().insert(why);
    };
    let behavioural = s.iter().any(|c| !NON_BEHAVIOURAL.contains(&c.as_str()));
    if !risk_known {
        // Conservative default without risk data: correctness and tests on behavioural clusters.
        if behavioural || s.is_empty() {
            enable(ReviewerKind::Correctness, "risk_unavailable");
            enable(ReviewerKind::Test, "risk_unavailable");
        }
        return on;
    }
    if behavioural {
        enable(ReviewerKind::Correctness, "behavioural_change");
        enable(ReviewerKind::Test, "behavioural_change");
    }
    if any(s, AUTH_SIGNALS) {
        for k in [
            ReviewerKind::Correctness,
            ReviewerKind::Security,
            ReviewerKind::Test,
            ReviewerKind::Architecture,
        ] {
            enable(k, "auth_change");
        }
    }
    if any(s, DATABASE_SIGNALS) {
        for k in [
            ReviewerKind::Correctness,
            ReviewerKind::Test,
            ReviewerKind::Architecture,
        ] {
            enable(k, "database_change");
        }
    }
    if any(s, API_SIGNALS) {
        enable(ReviewerKind::Correctness, "api_contract_change");
        enable(ReviewerKind::Test, "api_contract_change");
        if cluster.public_endpoint {
            enable(ReviewerKind::Security, "public_api_contract_change");
        }
    }
    if any(s, PERFORMANCE_SIGNALS) {
        enable(ReviewerKind::Performance, "hot_path_change");
    }
    if any(s, DEPENDENCY_SIGNALS) {
        enable(ReviewerKind::Security, "dependency_change");
        enable(ReviewerKind::Architecture, "dependency_change");
    }
    if any(s, COMPLEXITY_SIGNALS) {
        enable(ReviewerKind::Maintainability, "complexity_growth");
    }
    on
}

fn precedence(k: ReviewerKind) -> usize {
    PRECEDENCE
        .iter()
        .position(|p| *p == k)
        .unwrap_or(PRECEDENCE.len())
}

/// Plans reviewer runs over clusters (REV-002).
pub fn plan_reviewers(
    clusters: &[ChangeCluster],
    risk: Option<&RiskAssessment>,
    cfg: &ReviewersConfig,
    registry: &ReviewerRegistry,
) -> ReviewerPlan {
    let globs = generated_globs(&cfg.generated_ignore);
    // Head-revision config can never disable reviewers.
    let toggles: BTreeMap<ReviewerKind, bool> = match cfg.source {
        ConfigSource::Base => cfg.toggles.clone(),
        ConfigSource::Head => BTreeMap::new(),
    };
    let level = risk.map_or(RiskBand::Medium, |r| r.level);

    let mut plan = ReviewerPlan::default();
    let mut ordered: Vec<&ChangeCluster> = clusters.iter().collect();
    ordered.sort_by(|a, b| {
        b.risk_score
            .total_cmp(&a.risk_score)
            .then_with(|| a.cluster_id.cmp(&b.cluster_id))
    });

    let mut entries: Vec<(usize, PlanEntry)> = Vec::new();
    for (rank, cluster) in ordered.iter().enumerate() {
        let symbols: Vec<String> = cluster
            .symbols
            .iter()
            .filter(|s| !s.generated && !globs.is_match(&s.path))
            .map(|s| s.symbol_id.clone())
            .collect();
        if symbols.is_empty() && !cluster.symbols.is_empty() {
            plan.skipped.push(SkippedEntry {
                reviewer: None,
                cluster_id: cluster.cluster_id.clone(),
                reason: SkipReason::GeneratedOnly,
            });
            continue;
        }
        let enabled = rule_table(cluster, risk.is_some());
        if enabled.is_empty() {
            plan.skipped.push(SkippedEntry {
                reviewer: None,
                cluster_id: cluster.cluster_id.clone(),
                reason: SkipReason::LowRiskOnly,
            });
            continue;
        }
        for (kind, reasons) in enabled {
            let toggle = toggles.get(&kind).copied();
            // Maintainability runs only when explicitly enabled.
            let disabled = toggle == Some(false)
                || (kind == ReviewerKind::Maintainability && toggle != Some(true));
            let skip = if disabled {
                Some(SkipReason::DisabledByConfig)
            } else if !registry.is_implemented(kind) {
                Some(SkipReason::NotImplemented)
            } else {
                None
            };
            if let Some(reason) = skip {
                plan.skipped.push(SkippedEntry {
                    reviewer: Some(kind),
                    cluster_id: cluster.cluster_id.clone(),
                    reason,
                });
                continue;
            }
            let focus = if kind == ReviewerKind::Correctness {
                active_profiles(cluster.signals.iter().map(String::as_str))
            } else {
                Vec::new()
            };
            entries.push((
                rank,
                PlanEntry {
                    reviewer: kind,
                    cluster_id: cluster.cluster_id.clone(),
                    focus,
                    budget: ContextBudget::for_risk(level),
                    tier: tier_for(kind, risk),
                    reasons: reasons.into_iter().map(str::to_owned).collect(),
                    symbols: symbols.clone(),
                },
            ));
        }
    }
    entries.sort_by_key(|(rank, e)| (*rank, precedence(e.reviewer)));
    plan.entries = entries.into_iter().map(|(_, e)| e).collect();
    plan
}
