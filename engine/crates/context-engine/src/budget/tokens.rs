//! Deterministic, conservative token estimator.

/// `ceil(chars / 4) + 8` (8 tokens of per-item framing overhead).
pub fn estimate_tokens(text: &str) -> u32 {
    let chars = u32::try_from(text.chars().count()).unwrap_or(u32::MAX - 11);
    chars.saturating_add(3) / 4 + 8
}
