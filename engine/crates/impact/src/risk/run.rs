//! The risk stage as one call: detectors' signals plus the clustering, low-risk verdicts and
//! impact graph in, a [`RiskAssessment`] out (RISK-004).

use std::collections::BTreeMap;

use review_core::change::ChangeClusterKey;

use review_core::ids::SymbolKey;
use review_core::location::RepoPath;

use super::effects::EffectsConfig;
use super::low_risk::LowRiskReport;
use super::policy::RiskPolicy;
use super::score::{assess, ClusterMembers, RiskAssessment, ScoreInput};
use super::signal::RiskSignal;
use crate::cluster::{ClusterKind, Clustering};
use crate::graph::ImpactGraph;
use crate::input::ChangeSet;
use crate::metrics;

/// Inputs of [`assess_risk`].
pub struct RiskInputs<'a> {
    pub changes: &'a ChangeSet,
    pub impact: Option<&'a ImpactGraph>,
    pub clustering: Option<&'a Clustering>,
    pub policy: &'a RiskPolicy,
    pub effects: &'a EffectsConfig,
    pub low_risk: &'a LowRiskReport,
    /// The detectors' signals ([`detect_all`](super::detect_all)).
    pub signals: Vec<RiskSignal>,
}

impl std::fmt::Debug for RiskInputs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RiskInputs")
            .field("signals", &self.signals.len())
            .field("clusters", &self.clustering.map(|c| c.clusters.len()))
            .field("impact", &self.impact.is_some())
            .finish()
    }
}

/// The scoring view of a clustering: members, files, whether a member is untested and whether
/// anything in the cluster is more than low-risk.
pub fn cluster_members(
    clustering: &Clustering,
    impact: Option<&ImpactGraph>,
    low_risk: &LowRiskReport,
) -> BTreeMap<ChangeClusterKey, ClusterMembers> {
    clustering
        .clusters
        .iter()
        .map(|cluster| {
            let untested = impact.is_some_and(|graph| {
                cluster.members.iter().any(|member| {
                    graph
                        .symbol(*member)
                        .is_some_and(|symbol| symbol.untested && symbol.skipped.is_none())
                })
            });
            let non_cosmetic = cluster.kind != ClusterKind::LowRisk
                && (cluster
                    .members
                    .iter()
                    .any(|member| !low_risk.symbol_is_low_risk(member))
                    || cluster
                        .files
                        .iter()
                        .any(|file| !low_risk.file_is_low_risk(file)));
            (
                cluster.id,
                ClusterMembers {
                    symbols: cluster.members.iter().copied().collect(),
                    files: cluster.files.iter().cloned().collect(),
                    untested,
                    non_cosmetic,
                },
            )
        })
        .collect()
}

/// Scores the pull request, its clusters and its symbols and derives their effects.
pub fn assess_risk(inputs: RiskInputs<'_>) -> RiskAssessment {
    let span = tracing::info_span!(
        "risk_classification",
        pr_score = tracing::field::Empty,
        pr_level = tracing::field::Empty,
        signals = tracing::field::Empty,
    );
    let _entered = span.enter();

    let changes = inputs.changes;
    let clusters = inputs
        .clustering
        .map(|clustering| cluster_members(clustering, inputs.impact, inputs.low_risk))
        .unwrap_or_default();
    let symbol_files: BTreeMap<SymbolKey, RepoPath> = changes
        .symbols
        .iter()
        .map(|symbol| (symbol.key(), symbol.path().clone()))
        .collect();
    let has_untested = inputs.impact.is_some_and(|graph| {
        graph
            .symbols
            .iter()
            .any(|symbol| symbol.untested && symbol.skipped.is_none())
    });
    let non_cosmetic = changes
        .symbols
        .iter()
        .any(|symbol| !inputs.low_risk.symbol_is_low_risk(&symbol.key()))
        || changes
            .files
            .iter()
            .any(|file| !inputs.low_risk.file_is_low_risk(&file.path));
    let score_input = ScoreInput {
        signals: inputs.signals,
        clusters,
        low_risk_symbols: inputs
            .low_risk
            .per_symbol
            .iter()
            .filter(|(_, verdict)| verdict.is_low_risk())
            .map(|(key, _)| *key)
            .collect(),
        low_risk_files: inputs
            .low_risk
            .per_file
            .iter()
            .filter(|(_, verdict)| verdict.is_low_risk())
            .map(|(path, _)| path.clone())
            .collect(),
        files_changed: changes.files.len(),
        symbol_files,
        has_untested,
        non_cosmetic,
        effects_config: inputs.effects.clone(),
        change_input_hash: changes.input_hash.clone(),
        impact_input_hash: inputs
            .impact
            .map(|graph| graph.input_hash.clone())
            .unwrap_or_default(),
        policy_hash: inputs.policy.policy_hash(),
    };
    let assessment = assess(&score_input);

    span.record("pr_score", f64::from(assessment.score));
    span.record("pr_level", assessment.level.as_str());
    span.record("signals", assessment.signals.len() as u64);
    metrics::record_risk(assessment.score, assessment.level.as_str());
    assessment
}
