//! RISK-005 effects tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use impact::risk::effects::{
    effects, EffectTier, EffectsConfig, FocusProfile, ReviewDepth, RiskEffects, VerificationDepth,
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

#[test]
fn intent_not_an_input_compile_check() {
    // The effects of a cluster are a function of level, categories, test coverage, cosmetic-ness
    // and configuration only: assigning `effects` to this exact signature fails to compile if
    // another input (such as an intent assessment) is ever added.
    let _signature: fn(
        RiskLevel,
        &BTreeSet<RiskCategory>,
        bool,
        bool,
        &EffectsConfig,
    ) -> RiskEffects = effects;

    // Nor does any risk source file mention intent outside comments.
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/risk");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        // `rules.rs` renders the prose of the generated document, which states the invariant.
        let skip = path.extension().and_then(|e| e.to_str()) != Some("rs")
            || path.file_name().and_then(|n| n.to_str()) == Some("rules.rs");
        if skip {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            assert!(
                !code.to_ascii_lowercase().contains("intent"),
                "{}:{} mentions intent",
                path.display(),
                number + 1
            );
        }
    }
}

#[test]
fn effects_serialize_and_round_trip() {
    let e = effects(
        RiskLevel::High,
        &cats(&[RiskCategory::SchemaChanges]),
        true,
        true,
        &EffectsConfig::default(),
    );
    assert!(e.focus_profiles.contains(&FocusProfile::DatabaseSafety));
    let json = serde_json::to_string(&e).unwrap();
    let back: RiskEffects = serde_json::from_str(&json).unwrap();
    assert_eq!(back, e);
}
