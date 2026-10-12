//! RISK-005 effects tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use impact::risk::effects::{
    effects, EffectTier, EffectsConfig, FocusProfile, ReviewDepth, VerificationDepth,
};
use impact::risk::{RiskCategory, RiskLevel};
use review_core::reviewer_type::ReviewerType;

fn cats(items: &[RiskCategory]) -> BTreeSet<RiskCategory> {
    items.iter().copied().collect()
}

#[test]
fn auth_bypass_effects_security_deep_full_verification() {
    let e = effects(
        RiskLevel::Critical,
        &cats(&[RiskCategory::Authorization, RiskCategory::Authentication]),
        false,
        true,
        &EffectsConfig::default(),
    );
    assert!(e.reviewers.contains(&ReviewerType::Security));
    assert!(e.reviewers.contains(&ReviewerType::Correctness));
    assert_eq!(e.depth, ReviewDepth::Deep);
    assert_eq!(e.verification_depth, VerificationDepth::Full);
    assert_eq!(e.context_budget_multiplier, 1.5);
    assert_eq!(e.impact_budget_multiplier, 2.0);
    assert_eq!(e.model_tier, EffectTier::ReviewReasoner);
    assert!(e.test_inspection);
}

#[test]
fn low_risk_light_structural() {
    let e = effects(
        RiskLevel::Low,
        &BTreeSet::new(),
        true,
        false,
        &EffectsConfig::default(),
    );
    assert!(e.reviewers.is_empty());
    assert_eq!(e.depth, ReviewDepth::Light);
    assert_eq!(e.model_tier, EffectTier::FastReasoner);
    assert_eq!(e.verification_depth, VerificationDepth::Structural);
    assert_eq!(e.confidence_threshold_delta, 0.05);
    assert!(!e.test_inspection);
}

#[test]
fn db_signals_enable_database_safety_focus() {
    let e = effects(
        RiskLevel::High,
        &cats(&[RiskCategory::DatabaseMigrations]),
        false,
        true,
        &EffectsConfig::default(),
    );
    assert!(e.focus_profiles.contains(&FocusProfile::DatabaseSafety));
    assert!(e.reviewers.contains(&ReviewerType::Correctness));
    assert!(!e.reviewers.contains(&ReviewerType::Security));
}

#[test]
fn disabled_reviewer_recorded() {
    let mut config = EffectsConfig::default();
    config.enabled_reviewers.remove(&ReviewerType::Security);
    let e = effects(
        RiskLevel::Critical,
        &cats(&[RiskCategory::Secrets]),
        false,
        true,
        &config,
    );
    assert!(!e.reviewers.contains(&ReviewerType::Security));
    assert!(e.disabled_by_config.contains(&ReviewerType::Security));
}

#[test]
fn threshold_never_below_055() {
    let config = EffectsConfig {
        minimum_publish_confidence: 0.56,
        ..EffectsConfig::default()
    };
    let e = effects(RiskLevel::Critical, &BTreeSet::new(), false, true, &config);
    assert!(0.56 + e.confidence_threshold_delta >= 0.55 - 1e-6);
}

#[test]
fn authz_forces_full_verification_even_if_config_lowers() {
    let config = EffectsConfig {
        max_verification_depth: Some(VerificationDepth::Structural),
        ..EffectsConfig::default()
    };
    let e = effects(
        RiskLevel::High,
        &cats(&[RiskCategory::Authorization]),
        false,
        true,
        &config,
    );
    assert_eq!(e.verification_depth, VerificationDepth::Full);
    let other = effects(
        RiskLevel::High,
        &cats(&[RiskCategory::Concurrency]),
        false,
        true,
        &config,
    );
    assert_eq!(other.verification_depth, VerificationDepth::Structural);
}

#[test]
fn deep_reasoner_only_when_enabled_and_budgeted() {
    let config = EffectsConfig {
        deep_reasoner_enabled: true,
        deep_reasoner_budget_available: true,
        ..EffectsConfig::default()
    };
    assert!(effects(RiskLevel::Critical, &BTreeSet::new(), false, true, &config).deep_reasoner);
    assert!(
        !effects(
            RiskLevel::Critical,
            &BTreeSet::new(),
            false,
            true,
            &EffectsConfig::default()
        )
        .deep_reasoner
    );
}
