//! Risk levels (PRD §37–§38), shared by the risk engine, impact budgets and review planning.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A risk level. `Ord` follows severity: `Low < Medium < High < Critical`.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub const ALL: [RiskLevel; 4] = [Self::Low, Self::Medium, Self::High, Self::Critical];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    /// Level thresholds of RISK-004: `low < 0.25 ≤ medium < 0.50 ≤ high < 0.75 ≤ critical`.
    pub fn from_score(score: f32) -> Self {
        if score >= 0.75 {
            Self::Critical
        } else if score >= 0.5 {
            Self::High
        } else if score >= 0.25 {
            Self::Medium
        } else {
            Self::Low
        }
    }

    /// The lowest score of the level, used when a floor raises a level (RISK-004).
    pub const fn min_score(self) -> f32 {
        match self {
            Self::Low => 0.0,
            Self::Medium => 0.25,
            Self::High => 0.5,
            Self::Critical => 0.75,
        }
    }

    /// Parses the wire name.
    pub fn from_str_exact(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == raw)
    }
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_and_order() {
        assert_eq!(RiskLevel::from_score(0.0), RiskLevel::Low);
        assert_eq!(RiskLevel::from_score(0.2499), RiskLevel::Low);
        assert_eq!(RiskLevel::from_score(0.25), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_score(0.5), RiskLevel::High);
        assert_eq!(RiskLevel::from_score(0.75), RiskLevel::Critical);
        assert!(RiskLevel::Low < RiskLevel::Critical);
        for level in RiskLevel::ALL {
            assert_eq!(RiskLevel::from_score(level.min_score()), level);
            assert_eq!(RiskLevel::from_str_exact(level.as_str()), Some(level));
        }
    }
}
