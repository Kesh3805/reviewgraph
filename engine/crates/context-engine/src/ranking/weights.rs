//! Per-reviewer weight tables (version `v1`, from `weights/v1.yaml`) and their validation.

use std::collections::BTreeMap;
use std::sync::OnceLock;

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
    #[error("weights file does not parse: {0}")]
    Parse(String),
    #[error("weights file has no row for reviewer `{0}`")]
    MissingReviewer(&'static str),
}

impl Weights {
    /// Equal weights, used only if the embedded `v1` table fails to load.
    const UNIFORM: Self = Self {
        structural: 0.1,
        proximity: 0.1,
        execution_path: 0.1,
        changed_code_similarity: 0.1,
        test_relationship: 0.1,
        api_relationship: 0.1,
        config_relationship: 0.1,
        semantic_similarity: 0.1,
        historical: 0.1,
        risk_importance: 0.1,
    };

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

    /// The `v1` row of `weights/v1.yaml` for `reviewer`.
    pub fn for_reviewer(reviewer: ReviewerType) -> Self {
        static TABLE: OnceLock<Option<WeightTable>> = OnceLock::new();
        TABLE
            .get_or_init(|| WeightTable::v1().ok())
            .as_ref()
            .and_then(|t| t.get(reviewer))
            .unwrap_or(Self::UNIFORM)
    }
}

/// A versioned set of per-reviewer weights, loaded from YAML and validated row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct WeightTable {
    pub version: String,
    rows: BTreeMap<ReviewerType, Weights>,
}

#[derive(Deserialize)]
struct WeightFile {
    version: String,
    reviewers: BTreeMap<String, Weights>,
}

impl WeightTable {
    /// Parses a weights file. Every reviewer must have a row and every row must validate.
    pub fn from_yaml(text: &str) -> Result<Self, WeightError> {
        let file: WeightFile =
            serde_yaml::from_str(text).map_err(|e| WeightError::Parse(e.to_string()))?;
        let mut rows = BTreeMap::new();
        for reviewer in ReviewerType::ALL {
            let row = file
                .reviewers
                .get(reviewer.as_str())
                .ok_or(WeightError::MissingReviewer(reviewer.as_str()))?;
            row.validate()?;
            rows.insert(reviewer, *row);
        }
        Ok(Self {
            version: file.version,
            rows,
        })
    }

    /// The table shipped with the engine (`weights/v1.yaml`).
    pub fn v1() -> Result<Self, WeightError> {
        Self::from_yaml(include_str!("../../weights/v1.yaml"))
    }

    pub fn get(&self, reviewer: ReviewerType) -> Option<Weights> {
        self.rows.get(&reviewer).copied()
    }
}
