//! Correctness focus profiles (REV-002). "Database safety" from PRD §48 is a focus profile of the
//! correctness reviewer, not a seventh reviewer. Each active profile enables a prompt section.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusProfile {
    DatabaseSafety,
    AsyncSafety,
    ErrorHandling,
}

impl FocusProfile {
    pub const ALL: [FocusProfile; 3] =
        [Self::DatabaseSafety, Self::AsyncSafety, Self::ErrorHandling];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DatabaseSafety => "database_safety",
            Self::AsyncSafety => "async_safety",
            Self::ErrorHandling => "error_handling",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }

    /// Change or risk signals that activate this profile.
    pub const fn signals(self) -> &'static [&'static str] {
        match self {
            Self::DatabaseSafety => &[
                "database_write_changed",
                "migration_added",
                "transaction_boundary_changed",
            ],
            Self::AsyncSafety => &[
                "await_removed",
                "await_added",
                "promise_changed",
                "concurrency_changed",
            ],
            Self::ErrorHandling => &["throw_changed", "catch_changed", "error_handling_changed"],
        }
    }
}

/// The active profiles for a set of signals, sorted.
pub fn active_profiles<'a>(signals: impl IntoIterator<Item = &'a str>) -> Vec<FocusProfile> {
    let signals: BTreeSet<&str> = signals.into_iter().collect();
    FocusProfile::ALL
        .into_iter()
        .filter(|p| p.signals().iter().any(|s| signals.contains(s)))
        .collect()
}
