//! The convention confidence formula (PROF-003).
//!
//! `confidence = consistency x sample_factor x spread_factor`, where
//! `sample_factor = min(1, ln(1+n)/ln(1+30))` and `spread_factor = min(1, distinct_modules/3)`:
//! a pattern seen in one module only is weak evidence of an intentional convention.

/// Minimum confidence for a convention to be enforceable.
pub const ENFORCEABLE_CONFIDENCE: f32 = 0.9;
/// Minimum sample count for a convention to be enforceable.
pub const ENFORCEABLE_SAMPLES: u32 = 10;
/// Minimum consistency when narrowing a scope.
pub const SCOPE_CONSISTENCY: f32 = 0.9;

/// `conforming / (conforming + violating)`; `0.0` when there is no sample.
pub fn consistency(conforming: u32, violating: u32) -> f32 {
    let n = conforming + violating;
    if n == 0 {
        0.0
    } else {
        conforming as f32 / n as f32
    }
}

pub fn sample_factor(samples: u32) -> f32 {
    ((1.0 + samples as f32).ln() / (1.0 + 30.0_f32).ln()).min(1.0)
}

pub fn spread_factor(distinct_modules: u32) -> f32 {
    (distinct_modules as f32 / 3.0).min(1.0)
}

pub fn confidence(consistency: f32, samples: u32, distinct_modules: u32) -> f32 {
    consistency * sample_factor(samples) * spread_factor(distinct_modules)
}

/// Whether a convention with these numbers may be enforced (otherwise it is informational).
pub fn is_enforceable(confidence: f32, samples: u32) -> bool {
    confidence >= ENFORCEABLE_CONFIDENCE && samples >= ENFORCEABLE_SAMPLES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prd_example_spread_is_enforceable_single_module_is_not() {
        let c = consistency(37, 1);
        assert!(confidence(c, 38, 3) >= 0.9);
        assert!(confidence(c, 38, 1) < 0.9);
    }

    #[test]
    fn confidence_formula_table() {
        assert_eq!(consistency(0, 0), 0.0);
        assert_eq!(consistency(5, 5), 0.5);
        assert!((sample_factor(30) - 1.0).abs() < 1e-6);
        assert_eq!(sample_factor(1000), 1.0);
        assert_eq!(spread_factor(6), 1.0);
        assert!(!is_enforceable(0.95, 9));
        assert!(is_enforceable(0.95, 10));
    }

    #[test]
    fn monotonic_in_samples_and_consistency() {
        let mut prev = 0.0;
        for n in 0..60 {
            let c = confidence(0.95, n, 3);
            assert!(c >= prev);
            prev = c;
        }
        let mut prev = 0.0;
        for k in 0..=20 {
            let c = confidence(k as f32 / 20.0, 30, 3);
            assert!(c >= prev);
            prev = c;
        }
    }
}
