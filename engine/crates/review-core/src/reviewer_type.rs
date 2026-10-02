//! The kinds of reviewer ReviewGraph runs.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A reviewer specialisation. Wire form is snake_case. `Ord` gives a stable order for sorted,
/// deduplicated collections such as a run's degraded reviewers.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerType {
    Correctness,
    Security,
    Test,
    Architecture,
    Performance,
    Maintainability,
}

impl ReviewerType {
    pub const ALL: [ReviewerType; 6] = [
        Self::Correctness,
        Self::Security,
        Self::Test,
        Self::Architecture,
        Self::Performance,
        Self::Maintainability,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Correctness => "correctness",
            Self::Security => "security",
            Self::Test => "test",
            Self::Architecture => "architecture",
            Self::Performance => "performance",
            Self::Maintainability => "maintainability",
        }
    }
}

impl fmt::Display for ReviewerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_match_as_str() {
        for r in ReviewerType::ALL {
            assert_eq!(
                serde_json::to_string(&r).unwrap(),
                format!("\"{}\"", r.as_str())
            );
        }
        assert!(serde_json::from_str::<ReviewerType>("\"Security\"").is_err());
    }
}
