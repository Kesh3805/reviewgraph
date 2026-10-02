//! `Severity`, `FindingCategory` and `Confidence`.

use std::fmt;
use std::str::FromStr;

use schemars::gen::SchemaGenerator;
use schemars::schema::{InstanceType, Metadata, NumberValidation, Schema, SchemaObject};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::reviewer_type::ReviewerType;

/// Finding severity. `Ord` is **ascending** (`Info < Low < Medium < High < Critical`), so
/// `sev >= Severity::Medium` reads naturally for the PRD §55 band rule. The legacy order
/// (P0 < P1) was the reverse; ported tests must flip their comparisons.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub const ALL: [Severity; 5] = [
        Self::Info,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Critical,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    /// Maps a legacy label: P0 to Critical, P1 High, P2 Medium, P3 Low, P4 Info. Used only by the
    /// ported policy corpus.
    pub fn from_legacy(label: &str) -> Option<Severity> {
        match label {
            "P0" => Some(Self::Critical),
            "P1" => Some(Self::High),
            "P2" => Some(Self::Medium),
            "P3" => Some(Self::Low),
            "P4" => Some(Self::Info),
            _ => None,
        }
    }

    /// Accepts only the five lowercase wire values. `"HIGH"` is rejected, carrying over the legacy
    /// case-sensitivity defect guard (`core.rs:458-466`).
    pub fn parse(s: &str) -> Option<Severity> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What kind of problem a finding describes. Wire form is snake_case.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    Correctness,
    Security,
    Performance,
    Testing,
    Architecture,
    Maintainability,
    DataIntegrity,
    Concurrency,
    ApiContract,
    ErrorHandling,
}

impl FindingCategory {
    pub const ALL: [FindingCategory; 10] = [
        Self::Correctness,
        Self::Security,
        Self::Performance,
        Self::Testing,
        Self::Architecture,
        Self::Maintainability,
        Self::DataIntegrity,
        Self::Concurrency,
        Self::ApiContract,
        Self::ErrorHandling,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Correctness => "correctness",
            Self::Security => "security",
            Self::Performance => "performance",
            Self::Testing => "testing",
            Self::Architecture => "architecture",
            Self::Maintainability => "maintainability",
            Self::DataIntegrity => "data_integrity",
            Self::Concurrency => "concurrency",
            Self::ApiContract => "api_contract",
            Self::ErrorHandling => "error_handling",
        }
    }

    /// The reviewer that normally emits this category. A reviewer may emit any category.
    pub const fn primary_reviewer(self) -> ReviewerType {
        match self {
            Self::Correctness | Self::DataIntegrity | Self::Concurrency | Self::ErrorHandling => {
                ReviewerType::Correctness
            }
            Self::Security => ReviewerType::Security,
            Self::Performance => ReviewerType::Performance,
            Self::Testing => ReviewerType::Test,
            Self::Architecture | Self::ApiContract => ReviewerType::Architecture,
            Self::Maintainability => ReviewerType::Maintainability,
        }
    }
}

impl fmt::Display for FindingCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A probability-like score in `[0, 1]`; never NaN.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Confidence(f32);

impl Confidence {
    pub fn new(v: f32) -> Result<Self, CoreError> {
        if v.is_finite() && (0.0..=1.0).contains(&v) {
            Ok(Self(v))
        } else {
            Err(CoreError::OutOfRange {
                field: "confidence",
                value: v.to_string(),
            })
        }
    }

    pub const fn get(self) -> f32 {
        self.0
    }
}

impl FromStr for Confidence {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let v: f32 = s.parse().map_err(|_| CoreError::OutOfRange {
            field: "confidence",
            value: s.chars().take(32).collect(),
        })?;
        Self::new(v)
    }
}

impl<'de> Deserialize<'de> for Confidence {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = f32::deserialize(deserializer)?;
        Self::new(v).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Confidence {
    fn schema_name() -> String {
        "Confidence".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        SchemaObject {
            instance_type: Some(InstanceType::Number.into()),
            metadata: Some(Box::new(Metadata {
                title: Some("Confidence".to_owned()),
                ..Default::default()
            })),
            number: Some(Box::new(NumberValidation {
                minimum: Some(0.0),
                maximum: Some(1.0),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_order_ascending() {
        let mut sorted = Severity::ALL;
        sorted.sort();
        assert_eq!(sorted, Severity::ALL);
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);
        assert!(Severity::High >= Severity::Medium);
    }

    #[test]
    fn severity_rejects_uppercase_and_legacy_labels() {
        for bad in ["HIGH", "High", "P1", "", "critical ", "warning", "error"] {
            assert_eq!(Severity::parse(bad), None, "{bad}");
            assert!(
                serde_json::from_str::<Severity>(&format!("\"{bad}\"")).is_err(),
                "{bad}"
            );
        }
        for s in Severity::ALL {
            assert_eq!(Severity::parse(s.as_str()), Some(s));
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
    }

    #[test]
    fn legacy_p_mapping_1_to_1() {
        let expected = [
            ("P0", Severity::Critical),
            ("P1", Severity::High),
            ("P2", Severity::Medium),
            ("P3", Severity::Low),
            ("P4", Severity::Info),
        ];
        for (label, sev) in expected {
            assert_eq!(Severity::from_legacy(label), Some(sev));
        }
        let mapped: std::collections::BTreeSet<_> = expected
            .iter()
            .filter_map(|(l, _)| Severity::from_legacy(l))
            .collect();
        assert_eq!(mapped.len(), 5);
        for bad in ["P5", "p0", "", "high", "P"] {
            assert_eq!(Severity::from_legacy(bad), None, "{bad}");
        }
    }

    #[test]
    fn confidence_rejects_nan_and_out_of_range() {
        for bad in [-0.001, 1.001, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(Confidence::new(bad).is_err(), "{bad}");
        }
        for ok in [0.0, 0.5, 1.0] {
            assert_eq!(Confidence::new(ok).unwrap().get(), ok);
        }
        assert!(serde_json::from_str::<Confidence>("1.5").is_err());
        assert!(serde_json::from_str::<Confidence>("-0.1").is_err());
        assert_eq!(
            serde_json::from_str::<Confidence>("0.75").unwrap().get(),
            0.75
        );
        assert!("0.5".parse::<Confidence>().is_ok());
        assert!("NaN".parse::<Confidence>().is_err());
        assert!("abc".parse::<Confidence>().is_err());
    }

    #[test]
    fn category_wire_names_and_primary_reviewer() {
        for c in FindingCategory::ALL {
            assert_eq!(
                serde_json::to_string(&c).unwrap(),
                format!("\"{}\"", c.as_str())
            );
        }
        assert_eq!(
            FindingCategory::Testing.primary_reviewer(),
            ReviewerType::Test
        );
        assert_eq!(
            FindingCategory::Security.primary_reviewer(),
            ReviewerType::Security
        );
        assert_eq!(
            FindingCategory::ApiContract.primary_reviewer(),
            ReviewerType::Architecture
        );
    }
}
