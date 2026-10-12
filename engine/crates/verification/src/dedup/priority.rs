//! Finding prioritization score (DED-004, PRD §57). Orders findings; never changes severity.

use std::cmp::Ordering;

use review_core::finding::severity::Severity;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriorityWeights {
    pub severity: f32,
    pub confidence: f32,
    pub impact_domain: f32,
    pub public_exposure: f32,
    pub change_risk: f32,
    pub blast_radius: f32,
    pub test_gap: f32,
}

impl PriorityWeights {
    /// `priority_v1` table.
    pub const V1: PriorityWeights = PriorityWeights {
        severity: 0.35,
        confidence: 0.20,
        impact_domain: 0.15,
        public_exposure: 0.10,
        change_risk: 0.08,
        blast_radius: 0.07,
        test_gap: 0.05,
    };

    fn sum(&self) -> f32 {
        self.severity
            + self.confidence
            + self.impact_domain
            + self.public_exposure
            + self.change_risk
            + self.blast_radius
            + self.test_gap
    }

    /// Weights sum to 1.0, none above 0.40, blast radius at most 0.10 (anti-inflation guard).
    pub fn validate(&self) -> Result<(), String> {
        if (self.sum() - 1.0).abs() > 1e-4 {
            return Err(format!("weights sum to {}, expected 1.0", self.sum()));
        }
        let all = [
            self.severity,
            self.confidence,
            self.impact_domain,
            self.public_exposure,
            self.change_risk,
            self.blast_radius,
            self.test_gap,
        ];
        if all.iter().any(|w| *w > 0.40 || *w < 0.0) {
            return Err("a weight is outside 0.0..=0.40".into());
        }
        if self.blast_radius > 0.10 {
            return Err("blast_radius weight above 0.10".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriorityInputs {
    pub severity: Severity,
    /// 0..1
    pub confidence: f32,
    /// Distinct callers (saturates at 32 inside the score).
    pub callers: u32,
    pub public_exposure: f32,
    pub security_impact: f32,
    pub data_integrity_impact: f32,
    /// 1 = untested.
    pub test_gap: f32,
    pub change_risk: f32,
}

/// Clamp to 0..=1; NaN becomes 0 (missing optional input).
fn unit(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

fn severity_value(s: Severity) -> f32 {
    match s {
        Severity::Info => 0.0,
        Severity::Low => 0.25,
        Severity::Medium => 0.5,
        Severity::High => 0.75,
        Severity::Critical => 1.0,
    }
}

/// Score in `0.0..=1.0`, rounded to 4 decimals. Depends only on its own inputs.
pub fn priority_score(i: &PriorityInputs, w: &PriorityWeights) -> f32 {
    let blast = (((1 + i.callers.min(1_000_000)) as f32).log2() / 33f32.log2()).min(1.0);
    let impact = unit(i.security_impact).max(unit(i.data_integrity_impact));
    let raw = w.severity * severity_value(i.severity)
        + w.confidence * unit(i.confidence)
        + w.impact_domain * impact
        + w.public_exposure * unit(i.public_exposure)
        + w.change_risk * unit(i.change_risk)
        + w.blast_radius * blast
        + w.test_gap * unit(i.test_gap);
    (raw.clamp(0.0, 1.0) * 10_000.0).round() / 10_000.0
}

/// Total order: score desc, severity desc, confidence desc, identity_key asc.
pub struct Ranked<'a> {
    pub identity_key: &'a str,
    pub score: f32,
    pub severity: Severity,
    pub confidence: f32,
}

pub fn compare(a: &Ranked<'_>, b: &Ranked<'_>) -> Ordering {
    b.score
        .total_cmp(&a.score)
        .then(b.severity.cmp(&a.severity))
        .then(b.confidence.total_cmp(&a.confidence))
        .then(a.identity_key.cmp(b.identity_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PriorityInputs {
        PriorityInputs {
            severity: Severity::Medium,
            confidence: 0.8,
            callers: 1,
            public_exposure: 0.0,
            security_impact: 0.0,
            data_integrity_impact: 0.0,
            test_gap: 0.0,
            change_risk: 0.0,
        }
    }

    #[test]
    fn weights_valid_and_blast_guard() {
        assert!(PriorityWeights::V1.validate().is_ok());
        let mut w = PriorityWeights::V1;
        w.blast_radius = 0.2;
        w.severity = 0.22;
        assert!(w.validate().is_err());
    }

    #[test]
    fn caller_count_effect_capped() {
        let w = PriorityWeights::V1;
        let a = priority_score(&base(), &w);
        let b = priority_score(&PriorityInputs { callers: 1000, ..base() }, &w);
        assert!(b - a <= 0.07 + 1e-4);
    }

    #[test]
    fn high_severity_beats_many_callers() {
        let w = PriorityWeights::V1;
        let hi = priority_score(&PriorityInputs { severity: Severity::High, ..base() }, &w);
        let lo = priority_score(&PriorityInputs { severity: Severity::Low, callers: 500, ..base() }, &w);
        assert!(hi > lo);
    }

    #[test]
    fn nan_clamped_and_golden() {
        let w = PriorityWeights::V1;
        let s = priority_score(&PriorityInputs { test_gap: f32::NAN, ..base() }, &w);
        // 0.35*0.5 + 0.20*0.8 + 0.07*log2(2)/log2(33)
        let expect = 0.175 + 0.16 + 0.07 / 33f32.log2();
        assert!((s - expect).abs() < 1e-3);
    }

    #[test]
    fn untested_scores_higher() {
        let w = PriorityWeights::V1;
        assert!(priority_score(&PriorityInputs { test_gap: 1.0, ..base() }, &w) > priority_score(&base(), &w));
    }

    #[test]
    fn ordering_total() {
        let a = Ranked { identity_key: "a", score: 0.5, severity: Severity::High, confidence: 0.5 };
        let b = Ranked { identity_key: "b", score: 0.5, severity: Severity::High, confidence: 0.5 };
        assert_eq!(compare(&a, &b), Ordering::Less);
    }
}
