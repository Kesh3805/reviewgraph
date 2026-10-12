//! Risk effects (RISK-005): what a risk level and its categories change in the review.
//!
//! [`effects`] is pure and takes no intent: reviewer selection depends on risk only (CHG-009).

use std::collections::BTreeSet;

use review_core::reviewer_type::ReviewerType;
use serde::Serialize;

use super::level::RiskLevel;
use super::signal::RiskCategory;

/// The suppression floor the confidence threshold never crosses.
pub const CONFIDENCE_FLOOR: f32 = 0.55;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDepth {
    Light,
    Standard,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationDepth {
    /// Stages 1-4.
    Structural,
    /// Stages 1-7, contradiction check deterministic only.
    Standard,
    /// Stages 1-8 including the verifier model.
    Full,
}

/// The model tier of the reviewer calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectTier {
    FastReasoner,
    ReviewReasoner,
}

/// Extra review focus handed to a reviewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusProfile {
    DatabaseSafety,
    Authz,
}

/// Configuration inputs of the effects (mapped from POL-001 by the pipeline).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectsConfig {
    pub enabled_reviewers: BTreeSet<ReviewerType>,
    pub deep_reasoner_enabled: bool,
    /// The run budget has room for a deep-reasoner call.
    pub deep_reasoner_budget_available: bool,
    /// `confidence.minimum_publish`.
    pub minimum_publish_confidence: f32,
    /// A configured cap on verification depth; ignored for Authorization/Secrets risk.
    pub max_verification_depth: Option<VerificationDepth>,
}

impl Default for EffectsConfig {
    fn default() -> Self {
        Self {
            enabled_reviewers: ReviewerType::ALL.into_iter().collect(),
            deep_reasoner_enabled: false,
            deep_reasoner_budget_available: false,
            minimum_publish_confidence: 0.7,
            max_verification_depth: None,
        }
    }
}

/// What the risk of one cluster (or the PR) changes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RiskEffects {
    pub reviewers: BTreeSet<ReviewerType>,
    pub depth: ReviewDepth,
    pub context_budget_multiplier: f32,
    pub impact_budget_multiplier: f32,
    pub model_tier: EffectTier,
    pub deep_reasoner: bool,
    pub verification_depth: VerificationDepth,
    pub confidence_threshold_delta: f32,
    pub test_inspection: bool,
    pub focus_profiles: BTreeSet<FocusProfile>,
    /// Reviewers the effects wanted but the configuration disabled.
    pub disabled_by_config: BTreeSet<ReviewerType>,
}

/// Computes the effects of `level` with the categories present, whether changed symbols are
/// untested and whether the change is more than cosmetic.
pub fn effects(
    level: RiskLevel,
    categories: &BTreeSet<RiskCategory>,
    has_untested_symbols: bool,
    non_cosmetic: bool,
    config: &EffectsConfig,
) -> RiskEffects {
    let mut wanted: BTreeSet<ReviewerType> = BTreeSet::new();
    let mut focus = BTreeSet::new();

    if level != RiskLevel::Low || non_cosmetic {
        wanted.insert(ReviewerType::Correctness);
    }
    if level >= RiskLevel::High {
        for category in categories {
            match category {
                c if c.is_security() => {
                    wanted.insert(ReviewerType::Security);
                }
                RiskCategory::DatabaseMigrations
                | RiskCategory::SchemaChanges
                | RiskCategory::TransactionBoundaries => {
                    focus.insert(FocusProfile::DatabaseSafety);
                }
                RiskCategory::Concurrency | RiskCategory::Queues | RiskCategory::BackgroundJobs => {
                    wanted.insert(ReviewerType::Performance);
                }
                RiskCategory::PublicApiContracts => {
                    wanted.insert(ReviewerType::Architecture);
                }
                _ => {}
            }
        }
    }
    if level == RiskLevel::Critical {
        wanted.insert(ReviewerType::Security);
    }
    if has_untested_symbols && level != RiskLevel::Low {
        wanted.insert(ReviewerType::Test);
    }
    if categories.contains(&RiskCategory::Authorization) {
        focus.insert(FocusProfile::Authz);
    }

    let reviewers: BTreeSet<ReviewerType> = wanted
        .iter()
        .copied()
        .filter(|r| config.enabled_reviewers.contains(r))
        .collect();
    let disabled_by_config: BTreeSet<ReviewerType> = wanted
        .iter()
        .copied()
        .filter(|r| !config.enabled_reviewers.contains(r))
        .collect();

    let (depth, ctx, imp, tier, mut verification, delta, tests) = match level {
        RiskLevel::Low => (
            ReviewDepth::Light,
            0.5,
            0.5,
            EffectTier::FastReasoner,
            VerificationDepth::Structural,
            0.05,
            false,
        ),
        RiskLevel::Medium => (
            ReviewDepth::Standard,
            1.0,
            1.0,
            EffectTier::ReviewReasoner,
            VerificationDepth::Standard,
            0.0,
            has_untested_symbols,
        ),
        RiskLevel::High => (
            ReviewDepth::Deep,
            1.25,
            1.5,
            EffectTier::ReviewReasoner,
            VerificationDepth::Full,
            0.0,
            true,
        ),
        RiskLevel::Critical => (
            ReviewDepth::Deep,
            1.5,
            2.0,
            EffectTier::ReviewReasoner,
            VerificationDepth::Full,
            -0.03,
            true,
        ),
    };

    // A configuration may lower verification depth, but never for authorization/secrets risk.
    let forced_full = categories.contains(&RiskCategory::Authorization)
        || categories.contains(&RiskCategory::Secrets);
    if let Some(cap) = config.max_verification_depth {
        if !forced_full {
            verification = verification.min(cap);
        }
    }
    if forced_full && level >= RiskLevel::High {
        verification = VerificationDepth::Full;
    }

    // Never lower the effective threshold below the suppression floor.
    let min_delta = CONFIDENCE_FLOOR - config.minimum_publish_confidence;
    let confidence_threshold_delta = if delta < 0.0 {
        delta.max(min_delta.min(0.0))
    } else {
        delta
    };

    RiskEffects {
        reviewers,
        depth,
        context_budget_multiplier: ctx,
        impact_budget_multiplier: imp,
        model_tier: tier,
        deep_reasoner: level == RiskLevel::Critical
            && config.deep_reasoner_enabled
            && config.deep_reasoner_budget_available,
        verification_depth: verification,
        confidence_threshold_delta,
        test_inspection: tests,
        focus_profiles: focus,
        disabled_by_config,
    }
}
