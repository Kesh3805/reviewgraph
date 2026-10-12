//! Cluster risk ranking, budget allocation and the unreviewed-region report (IMP-010, PRD §91).
//!
//! Clusters are ranked by risk; the review budget is spent in that order. Critical and high
//! clusters (and anything carrying authorization, validation or secrets signals) are always
//! planned, even past the soft budget; medium clusters are planned while budget remains; low
//! clusters and the low-risk pseudo-cluster only when enough budget is left after the medium
//! ones. Everything not planned is reported as an [`UnreviewedRegion`], so completeness is
//! computed from the plan rather than self-reported: every changed file is in a planned unit
//! or in the unreviewed list.

use std::collections::BTreeSet;

use review_core::change::ChangeClusterKey;
use review_core::ids::SymbolKey;
use review_core::location::RepoPath;
use review_core::reviewer_type::ReviewerType;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ChangeCluster, ClusterKind, Clustering};
use crate::input::{ChangeSet, FileDisposition};
use crate::metrics;
use crate::risk::effects::{effects, EffectsConfig, RiskEffects};
use crate::risk::low_risk::{LowRiskKind, LowRiskReport};
use crate::risk::score::{RiskAssessment, ENDPOINT_REACHABLE};
use crate::risk::{RiskCategory, RiskLevel, RiskSignal, SignalSubject};

/// Warning emitted when a high or critical cluster is planned past the budget.
pub const BUDGET_OVERRUN_PLANNED: &str = "budget_overrun_planned";

/// Contribution at or above which a signal counts as a "high" signal of its cluster.
const HIGH_SIGNAL_CONTRIBUTION: f32 = 0.5;

/// Why a region of the change is not reviewed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UnreviewedReason {
    BudgetExhausted,
    LowRiskSkipped,
    GeneratedCode,
    Binary,
    ParseDegraded,
    TooLarge,
}

impl UnreviewedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BudgetExhausted => "budget_exhausted",
            Self::LowRiskSkipped => "low_risk_skipped",
            Self::GeneratedCode => "generated_code",
            Self::Binary => "binary",
            Self::ParseDegraded => "parse_degraded",
            Self::TooLarge => "too_large",
        }
    }

    /// The label used in the summary line.
    pub const fn label(self) -> &'static str {
        match self {
            Self::BudgetExhausted => "budget exhausted",
            Self::LowRiskSkipped => "low risk",
            Self::GeneratedCode => "generated code",
            Self::Binary => "binary files",
            Self::ParseDegraded => "parse degraded",
            Self::TooLarge => "too large",
        }
    }
}

fn kind_label(kind: LowRiskKind) -> &'static str {
    match kind {
        LowRiskKind::FormattingOrComments => "formatting",
        LowRiskKind::PureRename => "renames",
        LowRiskKind::Generated => "generated code",
        LowRiskKind::ImportSort => "import sorting",
        LowRiskKind::SimpleConstant => "constant changes",
        LowRiskKind::SnapshotUpdate => "snapshot updates",
    }
}

/// The run budget the plan allocates from (PIPE-006 clamps it at run time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanBudget {
    pub model_tokens: u64,
    pub model_calls: u32,
    /// `review.max_symbols`.
    pub reviewed_symbols: u32,
}

impl Default for PlanBudget {
    fn default() -> Self {
        Self {
            model_tokens: 300_000,
            model_calls: 60,
            reviewed_symbols: 100,
        }
    }
}

/// Planning configuration.
#[derive(Debug, Clone)]
pub struct PlanConfig {
    pub budget: PlanBudget,
    /// Tokens reserved for each reviewer's output.
    pub output_reserve_tokens: u32,
    /// The context cap of one reviewer at multiplier 1.0.
    pub base_context_tokens: u32,
    /// Low clusters are planned only when at least this fraction of the token budget is left
    /// after the medium clusters.
    pub low_risk_budget_fraction: f32,
    /// Configuration of the per-cluster effects used when the assessment carries none.
    pub effects: EffectsConfig,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            budget: PlanBudget::default(),
            output_reserve_tokens: 2_000,
            base_context_tokens: 8_000,
            low_risk_budget_fraction: 0.3,
            effects: EffectsConfig::default(),
        }
    }
}

/// One cluster the review will run on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlannedUnit {
    pub cluster_id: ChangeClusterKey,
    /// 0 is reviewed first.
    pub priority: u32,
    pub risk_level: RiskLevel,
    pub reviewers_hint: BTreeSet<ReviewerType>,
    pub token_allowance: u32,
    /// The unit was planned although it does not fit the remaining budget.
    pub over_budget: bool,
    pub files: Vec<RepoPath>,
    pub symbols: Vec<SymbolKey>,
}

/// A region of the change the review does not cover, with the reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnreviewedRegion {
    /// `None` for files outside every cluster (binary, generated, oversized).
    pub cluster_id: Option<ChangeClusterKey>,
    pub files: Vec<RepoPath>,
    pub symbols: Vec<SymbolKey>,
    pub risk_level: RiskLevel,
    pub reason: UnreviewedReason,
    /// The low-risk kinds behind a `LowRiskSkipped` region.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<LowRiskKind>,
}

/// The output of planning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewPlan {
    /// In review order.
    pub units: Vec<PlannedUnit>,
    pub unreviewed: Vec<UnreviewedRegion>,
    pub clusters_total: u32,
    /// A critical or high unit was planned past the budget.
    pub over_budget: bool,
    pub warnings: Vec<String>,
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 {
        one
    } else {
        many
    }
}

impl ReviewPlan {
    /// Every changed file is in a planned unit or in an unreviewed region.
    pub fn covers_all(&self, change: &ChangeSet) -> bool {
        self.uncovered_files(change).is_empty()
    }

    /// The changed files that appear in neither a planned unit nor an unreviewed region.
    pub fn uncovered_files(&self, change: &ChangeSet) -> Vec<RepoPath> {
        let covered: BTreeSet<&RepoPath> = self
            .units
            .iter()
            .flat_map(|unit| unit.files.iter())
            .chain(
                self.unreviewed
                    .iter()
                    .flat_map(|region| region.files.iter()),
            )
            .collect();
        change
            .files
            .iter()
            .map(|file| &file.path)
            .filter(|path| !covered.contains(path))
            .cloned()
            .collect()
    }

    /// The disclosure line for the review summary (GH-008), e.g. `Reviewed 4 of 6 change
    /// clusters (all high/critical). Skipped: 2 low-risk clusters (14 files: formatting, import
    /// sorting).`
    pub fn summary_line(&self) -> String {
        let mut line = format!(
            "Reviewed {} of {} change clusters",
            self.units.len(),
            self.clusters_total
        );
        if !self.units.is_empty()
            && self
                .units
                .iter()
                .all(|unit| unit.risk_level >= RiskLevel::High)
        {
            line.push_str(" (all high/critical)");
        }
        line.push('.');

        let skipped: Vec<&UnreviewedRegion> = self
            .unreviewed
            .iter()
            .filter(|region| region.cluster_id.is_some())
            .collect();
        if !skipped.is_empty() {
            let all_low = skipped
                .iter()
                .all(|region| region.reason == UnreviewedReason::LowRiskSkipped);
            let files: BTreeSet<&RepoPath> = skipped
                .iter()
                .flat_map(|region| region.files.iter())
                .collect();
            let labels = labels_of(&skipped);
            line.push_str(&format!(
                " Skipped: {} {}{} ({} {}: {}).",
                skipped.len(),
                if all_low { "low-risk " } else { "" },
                plural(skipped.len(), "cluster", "clusters"),
                files.len(),
                plural(files.len(), "file", "files"),
                labels.join(", ")
            ));
        }

        let others: Vec<&UnreviewedRegion> = self
            .unreviewed
            .iter()
            .filter(|region| region.cluster_id.is_none())
            .collect();
        if !others.is_empty() {
            let files: BTreeSet<&RepoPath> = others
                .iter()
                .flat_map(|region| region.files.iter())
                .collect();
            line.push_str(&format!(
                " {} other {} not reviewed ({}).",
                files.len(),
                plural(files.len(), "file", "files"),
                labels_of(&others).join(", ")
            ));
        }
        line
    }
}

/// Sorted, distinct labels of the regions: their low-risk kinds, else their reasons.
fn labels_of(regions: &[&UnreviewedRegion]) -> Vec<&'static str> {
    let labels: BTreeSet<&'static str> = regions
        .iter()
        .flat_map(|region| {
            if region.kinds.is_empty() {
                vec![region.reason.label()]
            } else {
                region
                    .kinds
                    .iter()
                    .map(|kind| kind_label(*kind))
                    .collect::<Vec<&'static str>>()
            }
        })
        .collect();
    labels.into_iter().collect()
}

/// Inputs of [`plan_review`].
#[derive(Clone, Copy)]
pub struct PlanInputs<'a> {
    pub change: &'a ChangeSet,
    pub clustering: &'a Clustering,
    pub assessment: &'a RiskAssessment,
    pub low_risk: Option<&'a LowRiskReport>,
    pub config: &'a PlanConfig,
}

impl std::fmt::Debug for PlanInputs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanInputs")
            .field("files", &self.change.files.len())
            .field("clusters", &self.clustering.clusters.len())
            .field("risk_level", &self.assessment.level)
            .finish()
    }
}

/// A cluster with its planning attributes.
struct Ranked<'a> {
    cluster: &'a ChangeCluster,
    score: f32,
    level: RiskLevel,
    effects: RiskEffects,
    /// 0-based position in the ranking.
    rank: u32,
}

fn in_scope(signal: &RiskSignal, cluster: &ChangeCluster) -> bool {
    match &signal.subject {
        SignalSubject::Symbol(key) => cluster.members.contains(key),
        SignalSubject::File(path) => cluster.files.contains(path),
        SignalSubject::Cluster(id) => *id == cluster.id,
        SignalSubject::Pr => false,
    }
}

fn rank_cluster<'a>(inputs: &PlanInputs<'a>, cluster: &'a ChangeCluster) -> Ranked<'a> {
    let assessment = inputs.assessment;
    let scope: Vec<&RiskSignal> = assessment
        .signals
        .iter()
        .filter(|signal| in_scope(signal, cluster))
        .collect();

    let mut base = 0.0f32;
    let mut level = RiskLevel::Low;
    if let Some(scored) = assessment.per_cluster.get(&cluster.id) {
        base = base.max(scored.score);
        level = level.max(scored.level);
    }
    for member in &cluster.members {
        if let Some(scored) = assessment.per_symbol.get(member) {
            base = base.max(scored.score);
            level = level.max(scored.level);
        }
    }
    let high_signals = scope
        .iter()
        .filter(|signal| signal.contribution >= HIGH_SIGNAL_CONTRIBUTION)
        .count();
    let reachable = scope.iter().any(|s| s.rule_id == ENDPOINT_REACHABLE);
    let score =
        (base + 0.1 * (high_signals as f32 / 3.0).min(1.0) + if reachable { 0.05 } else { 0.0 })
            .min(1.0);

    // Authorization, validation and secrets risk can never be skipped for budget.
    let security_sensitive = scope.iter().any(|signal| {
        matches!(
            signal.category,
            RiskCategory::Authorization | RiskCategory::Validation | RiskCategory::Secrets
        )
    });
    if security_sensitive {
        level = level.max(RiskLevel::High);
    }

    let cluster_effects = assessment
        .cluster_effects
        .get(&cluster.id)
        .cloned()
        .unwrap_or_else(|| {
            let categories: BTreeSet<RiskCategory> =
                scope.iter().map(|signal| signal.category).collect();
            effects(level, &categories, false, true, &inputs.config.effects)
        });
    Ranked {
        cluster,
        score,
        level,
        effects: cluster_effects,
        rank: 0,
    }
}

fn unit_cost(config: &PlanConfig, effects: &RiskEffects) -> (u32, u32) {
    let cap = (config.base_context_tokens as f32 * effects.context_budget_multiplier).round();
    let cap = if cap.is_finite() && cap > 0.0 {
        cap as u32
    } else {
        0
    };
    let reviewers = u32::try_from(effects.reviewers.len().max(1)).unwrap_or(u32::MAX);
    let per_reviewer = cap.saturating_add(config.output_reserve_tokens);
    (per_reviewer.saturating_mul(reviewers), reviewers)
}

fn low_risk_kinds(cluster: &ChangeCluster, low_risk: Option<&LowRiskReport>) -> Vec<LowRiskKind> {
    let Some(report) = low_risk else {
        return Vec::new();
    };
    let kinds: BTreeSet<LowRiskKind> = cluster
        .members
        .iter()
        .filter_map(|member| report.per_symbol.get(member).and_then(|v| v.kind))
        .chain(
            cluster
                .files
                .iter()
                .filter_map(|file| report.per_file.get(file).and_then(|v| v.kind)),
        )
        .collect();
    kinds.into_iter().collect()
}

/// Ranks the clusters and allocates the review budget in that order.
pub fn plan_review(inputs: &PlanInputs<'_>) -> ReviewPlan {
    let span = tracing::info_span!(
        "review_planning",
        clusters_total = tracing::field::Empty,
        clusters_planned = tracing::field::Empty,
        clusters_skipped = tracing::field::Empty,
        over_budget = tracing::field::Empty,
    );
    let _entered = span.enter();

    let config = inputs.config;
    let mut ranked: Vec<Ranked<'_>> = inputs
        .clustering
        .clusters
        .iter()
        .map(|cluster| rank_cluster(inputs, cluster))
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.cluster.size_lines.cmp(&a.cluster.size_lines))
            .then_with(|| a.cluster.id.cmp(&b.cluster.id))
    });
    for (position, entry) in ranked.iter_mut().enumerate() {
        entry.rank = u32::try_from(position).unwrap_or(u32::MAX);
    }

    let total_tokens = config.budget.model_tokens;
    let mut tokens_left = total_tokens;
    let mut calls_left = config.budget.model_calls;
    let mut symbols_left = config.budget.reviewed_symbols as usize;
    let mut over_budget = false;
    let mut warnings: Vec<String> = Vec::new();
    let mut units: Vec<(u32, PlannedUnit)> = Vec::new();
    let mut regions: Vec<UnreviewedRegion> = Vec::new();

    // Phase 1: critical and high (and security-sensitive) clusters, always planned.
    // Phase 2: medium clusters while budget remains.
    // Phase 3: low clusters and the low-risk pseudo-cluster, only with budget to spare.
    for phase in 1..=3u8 {
        for entry in &ranked {
            let is_pseudo = entry.cluster.kind == ClusterKind::LowRisk;
            let entry_phase = if is_pseudo {
                3
            } else if entry.level >= RiskLevel::High {
                1
            } else if entry.level == RiskLevel::Medium {
                2
            } else {
                3
            };
            if entry_phase != phase {
                continue;
            }
            let (cost, reviewers) = unit_cost(config, &entry.effects);
            let members = entry.cluster.members.len();
            let cost64 = u64::from(cost);
            let fits = cost64 <= tokens_left && reviewers <= calls_left && members <= symbols_left;
            let enough_spare = phase != 3
                || (tokens_left as f64)
                    >= f64::from(config.low_risk_budget_fraction) * (total_tokens as f64);
            let plan_it = phase == 1 || (fits && enough_spare);
            if plan_it {
                let over = phase == 1 && (cost64 > tokens_left || reviewers > calls_left);
                if over {
                    over_budget = true;
                    if !warnings.iter().any(|w| w == BUDGET_OVERRUN_PLANNED) {
                        warnings.push(BUDGET_OVERRUN_PLANNED.to_owned());
                    }
                }
                tokens_left = tokens_left.saturating_sub(cost64);
                calls_left = calls_left.saturating_sub(reviewers);
                symbols_left = symbols_left.saturating_sub(members);
                units.push((
                    entry.rank,
                    PlannedUnit {
                        cluster_id: entry.cluster.id,
                        priority: 0,
                        risk_level: entry.level,
                        reviewers_hint: entry.effects.reviewers.clone(),
                        token_allowance: cost,
                        over_budget: over,
                        files: entry.cluster.files.clone(),
                        symbols: entry.cluster.members.clone(),
                    },
                ));
            } else {
                let degraded = !entry.cluster.members.is_empty()
                    && entry.cluster.members.iter().all(|member| {
                        inputs
                            .change
                            .symbol(*member)
                            .is_some_and(|symbol| symbol.parse_degraded)
                    });
                let reason = if degraded {
                    UnreviewedReason::ParseDegraded
                } else if phase == 3 {
                    UnreviewedReason::LowRiskSkipped
                } else {
                    UnreviewedReason::BudgetExhausted
                };
                regions.push(UnreviewedRegion {
                    cluster_id: Some(entry.cluster.id),
                    files: entry.cluster.files.clone(),
                    symbols: entry.cluster.members.clone(),
                    risk_level: entry.level,
                    reason,
                    kinds: if reason == UnreviewedReason::LowRiskSkipped {
                        low_risk_kinds(entry.cluster, inputs.low_risk)
                    } else {
                        Vec::new()
                    },
                });
            }
        }
    }

    // Units are listed in rank order, and the rank is their priority.
    units.sort_by_key(|(rank, _)| *rank);
    let units: Vec<PlannedUnit> = units
        .into_iter()
        .enumerate()
        .map(|(position, (_, mut unit))| {
            unit.priority = u32::try_from(position).unwrap_or(u32::MAX);
            unit
        })
        .collect();

    // Files outside every planned cluster: reported by their disposition.
    let planned_files: BTreeSet<&RepoPath> =
        units.iter().flat_map(|unit| unit.files.iter()).collect();
    for region in &mut regions {
        region.files.retain(|file| !planned_files.contains(file));
    }
    regions.retain(|region| !region.files.is_empty() || !region.symbols.is_empty());
    let listed: BTreeSet<RepoPath> = regions
        .iter()
        .flat_map(|region| region.files.iter().cloned())
        .collect();
    for file in &inputs.change.files {
        if planned_files.contains(&file.path) || listed.contains(&file.path) {
            continue;
        }
        let reason = match file.disposition {
            FileDisposition::Generated | FileDisposition::Vendored | FileDisposition::Minified => {
                UnreviewedReason::GeneratedCode
            }
            FileDisposition::Binary => UnreviewedReason::Binary,
            FileDisposition::Large => UnreviewedReason::TooLarge,
            FileDisposition::Analyzed if file.parse_degraded => UnreviewedReason::ParseDegraded,
            FileDisposition::Analyzed => UnreviewedReason::LowRiskSkipped,
        };
        let kinds = inputs
            .low_risk
            .and_then(|report| report.per_file.get(&file.path))
            .and_then(|verdict| verdict.kind)
            .into_iter()
            .collect();
        regions.push(UnreviewedRegion {
            cluster_id: None,
            files: vec![file.path.clone()],
            symbols: Vec::new(),
            risk_level: RiskLevel::Low,
            reason,
            kinds,
        });
    }

    for unit in &units {
        metrics::record_unit_planned(unit.risk_level.as_str());
    }
    for region in &regions {
        metrics::record_unreviewed(region.reason.as_str());
    }
    let skipped_clusters = regions
        .iter()
        .filter(|region| region.cluster_id.is_some())
        .count();
    span.record("clusters_total", ranked.len() as u64);
    span.record("clusters_planned", units.len() as u64);
    span.record("clusters_skipped", skipped_clusters as u64);
    span.record("over_budget", over_budget);

    ReviewPlan {
        units,
        unreviewed: regions,
        clusters_total: u32::try_from(ranked.len()).unwrap_or(u32::MAX),
        over_budget,
        warnings,
    }
}
