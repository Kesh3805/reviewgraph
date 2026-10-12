//! Budget resolution: defaults, risk multiplier, config clamp, run budget.

use review_core::reviewer_type::ReviewerType;
use serde::{Deserialize, Serialize};

/// Hard maximum tokens per package; config cannot exceed it.
pub const HARD_MAX_TOKENS: u32 = 150_000;
/// Reserved for instructions and the output schema.
pub const RESERVE_TOKENS: u32 = 2_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimits {
    pub max_changed: u16,
    pub max_related: u16,
    pub max_tests: u16,
    pub max_configs: u16,
    pub max_rule_docs: u16,
    /// Tokens available for items (reserve already subtracted).
    pub max_tokens: u32,
    pub reserve_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BudgetError {
    #[error("context budget exhausted")]
    BudgetExhausted,
}

/// `(max_changed, max_related, max_tests, max_configs, max_rule_docs, default token cap)`.
fn defaults(reviewer: ReviewerType) -> (u16, u16, u16, u16, u16, u32) {
    match reviewer {
        ReviewerType::Correctness => (8, 20, 8, 4, 4, 24_000),
        ReviewerType::Security => (8, 16, 6, 6, 6, 24_000),
        ReviewerType::Test => (8, 10, 20, 2, 4, 20_000),
        ReviewerType::Architecture => (8, 24, 4, 6, 8, 24_000),
        ReviewerType::Performance => (6, 16, 4, 4, 2, 20_000),
        ReviewerType::Maintainability => (8, 16, 6, 2, 8, 18_000),
    }
}

fn scale(count: u16, multiplier: f32) -> u16 {
    let v = (f32::from(count) * multiplier).ceil();
    if v.is_nan() || v < 1.0 {
        1
    } else if v > f32::from(u16::MAX) {
        u16::MAX
    } else {
        v as u16
    }
}

/// `max_tokens = min(round(default_cap x multiplier), config_max, run_remaining, hard max) - reserve`.
/// Related and test counts scale by the multiplier (`ceil`); changed symbols never drop below 1.
pub fn resolve(
    reviewer: ReviewerType,
    multiplier: f32,
    config_max_tokens: u32,
    run_remaining_tokens: u32,
) -> Result<BudgetLimits, BudgetError> {
    let m = if multiplier.is_nan() || multiplier <= 0.0 {
        1.0
    } else {
        multiplier
    };
    let (changed, related, tests, configs, rules, cap) = defaults(reviewer);
    let scaled_cap = (cap as f32 * m).round().min(HARD_MAX_TOKENS as f32) as u32;
    let total = scaled_cap
        .min(config_max_tokens)
        .min(run_remaining_tokens)
        .min(HARD_MAX_TOKENS);
    let max_tokens = total.saturating_sub(RESERVE_TOKENS);
    if max_tokens == 0 {
        return Err(BudgetError::BudgetExhausted);
    }
    Ok(BudgetLimits {
        max_changed: changed.max(1),
        max_related: scale(related, m),
        max_tests: scale(tests, m),
        max_configs: configs,
        max_rule_docs: rules,
        max_tokens,
        reserve_tokens: RESERVE_TOKENS,
    })
}
