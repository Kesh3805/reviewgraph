//! Per-reviewer weight tables (version `v1`) and their validation.

use review_core::reviewer_type::ReviewerType;
use serde::{Deserialize, Serialize};

pub const WEIGHTS_VERSION: &str = "v1";
/// No single signal may carry more than this weight.
pub const MAX_WEIGHT: f32 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weights {
    pub structural: f32,
    pub proximity: f32,
    pub execution_path: f32,
    pub changed_code_similarity: f32,
    pub test_relationship: f32,
    pub api_relationship: f32,
    pub config_relationship: f32,
    pub semantic_similarity: f32,
    pub historical: f32,
    pub risk_importance: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum WeightError {
    #[error("weight `{field}` = {value} is outside [0, {MAX_WEIGHT}]")]
    OutOfRange { field: &'static str, value: f32 },
    #[error("weights sum to {sum}, expected 1.0")]
    BadSum { sum: f64 },
}

impl Weights {
    fn fields(&self) -> [(&'static str, f32); 10] {
        [
            ("structural", self.structural),
            ("proximity", self.proximity),
            ("execution_path", self.execution_path),
            ("changed_code_similarity", self.changed_code_similarity),
            ("test_relationship", self.test_relationship),
            ("api_relationship", self.api_relationship),
            ("config_relationship", self.config_relationship),
            ("semantic_similarity", self.semantic_similarity),
            ("historical", self.historical),
            ("risk_importance", self.risk_importance),
        ]
    }

    /// Every weight in `[0, 0.35]` and the row sums to 1.0 (tolerance 1e-5).
    pub fn validate(&self) -> Result<(), WeightError> {
        let mut sum = 0.0f64;
        for (field, value) in self.fields() {
            if value.is_nan() || !(0.0..=MAX_WEIGHT).contains(&value) {
                return Err(WeightError::OutOfRange { field, value });
            }
            sum += f64::from(value);
        }
        if (sum - 1.0).abs() > 1e-5 {
            return Err(WeightError::BadSum { sum });
        }
        Ok(())
    }

    /// The `v1` table. Column order: struct, prox, exec, sim, test, api, config, sem, hist, risk.
    pub fn for_reviewer(reviewer: ReviewerType) -> Self {
        let r = |a, b, c, d, e, f, g, h, i, j| Self {
            structural: a,
            proximity: b,
            execution_path: c,
            changed_code_similarity: d,
            test_relationship: e,
            api_relationship: f,
            config_relationship: g,
            semantic_similarity: h,
            historical: i,
            risk_importance: j,
        };
        match reviewer {
            ReviewerType::Correctness => {
                r(0.25, 0.20, 0.12, 0.10, 0.08, 0.06, 0.05, 0.06, 0.0, 0.08)
            }
            ReviewerType::Security => r(0.20, 0.15, 0.25, 0.05, 0.03, 0.12, 0.05, 0.03, 0.0, 0.12),
            ReviewerType::Test => r(0.15, 0.15, 0.05, 0.10, 0.35, 0.05, 0.03, 0.07, 0.0, 0.05),
            ReviewerType::Architecture => {
                r(0.30, 0.10, 0.05, 0.05, 0.0, 0.15, 0.10, 0.15, 0.0, 0.10)
            }
            ReviewerType::Performance => {
                r(0.25, 0.20, 0.15, 0.10, 0.02, 0.05, 0.08, 0.05, 0.0, 0.10)
            }
            ReviewerType::Maintainability => {
                r(0.25, 0.20, 0.0, 0.20, 0.05, 0.0, 0.05, 0.20, 0.0, 0.05)
            }
        }
    }
}
