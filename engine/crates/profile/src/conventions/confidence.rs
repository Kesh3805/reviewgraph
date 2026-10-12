//! The convention confidence formula (PROF-003, PRD §63).
//!
//! `confidence = consistency × sample_factor × spread_factor`, where
//! `sample_factor = min(1, ln(1+n) / ln(1+30))` and `spread_factor = min(1, distinct_modules / 3)`.
//! A pattern seen in a single module is weak however consistent it is. A convention is
//! enforceable only when `confidence ≥ 0.9` and `samples ≥ 10` (target-architecture §3.10).

/// Lowest confidence an enforceable convention may have.
pub const ENFORCEABLE_CONFIDENCE: f32 = 0.9;
/// Fewest samples an enforceable convention may have.
pub const ENFORCEABLE_SAMPLES: u32 = 10;
/// Lowest consistency a scope may have to be chosen by scope narrowing.
pub const SCOPE_CONSISTENCY: f64 = 0.9;

/// `conforming / (conforming + violating)`; zero when there is nothing to judge.
pub fn consistency(conforming: u32, violating: u32) -> f64 {
    let total = f64::from(conforming) + f64::from(violating);
    if total == 0.0 {
        0.0
    } else {
        f64::from(conforming) / total
    }
}

pub fn sample_factor(samples: u32) -> f64 {
    ((1.0 + f64::from(samples)).ln() / 31f64.ln()).min(1.0)
}

pub fn spread_factor(distinct_modules: u32) -> f64 {
    (f64::from(distinct_modules) / 3.0).min(1.0)
}

/// The PROF-003 confidence of a convention.
pub fn confidence(consistency: f64, samples: u32, distinct_modules: u32) -> f64 {
    (consistency.clamp(0.0, 1.0) * sample_factor(samples) * spread_factor(distinct_modules))
        .clamp(0.0, 1.0)
}

pub fn is_enforceable(confidence: f32, samples: u32) -> bool {
    confidence >= ENFORCEABLE_CONFIDENCE && samples >= ENFORCEABLE_SAMPLES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prd_63_example() {
        // 38 service methods, 37 conform.
        let c = consistency(37, 1);
        assert!(confidence(c, 38, 3) >= 0.9);
        assert!(confidence(c, 38, 1) < 0.9);
    }
}
