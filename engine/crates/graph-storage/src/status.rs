//! The snapshot status state machine shared by every adapter (GS-001).
//!
//! ```text
//!                +-----------+      +-------------+      +----------+
//!   Pending ---> | Indexing  | ---> | Persisting  | ---> |  Ready   |
//!      |         +-----------+      +-------------+      +----------+
//!      |                |                 |                    |
//!      |                +--------+--------+                    |
//!      |                         |                             v
//!      v                         v                        +-------------+
//!  +--------+ <------------------+--------------------+    | Inconsistent |
//!  | Failed |                                           +-> +-------------+
//!  +--------+
//! ```
//!
//! `transition` is a compare-and-swap: it returns `Ok(true)` only for the caller that moved the
//! row, `Ok(false)` when the row was not in `from` or the edge does not exist (so retries are
//! safe and never error), and `Err(StoreError::NotFound)` when the snapshot is not visible to
//! the caller's scope.
//!
//! Readers only ever see `Ready` snapshots.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lifecycle of one snapshot (`snapshots.status`, GS-003).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotStatus {
    /// Row exists; nothing has been written yet.
    Pending,
    /// A worker claimed the snapshot and is producing the payload.
    Indexing,
    /// The payload is being persisted. The only state in which `write_full`/`write_delta` run.
    Persisting,
    /// Fully materialized and safe to read.
    Ready,
    /// Writing or indexing failed; kept for diagnostics and reaped by retention.
    Failed,
    /// The consistency validator found a mismatch (ADR-004); readers must rebuild.
    Inconsistent,
}

impl SnapshotStatus {
    /// Every variant, in lifecycle order.
    pub const ALL: [SnapshotStatus; 6] = [
        SnapshotStatus::Pending,
        SnapshotStatus::Indexing,
        SnapshotStatus::Persisting,
        SnapshotStatus::Ready,
        SnapshotStatus::Failed,
        SnapshotStatus::Inconsistent,
    ];

    /// The exact text stored in `snapshots.status`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Indexing => "indexing",
            Self::Persisting => "persisting",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Inconsistent => "inconsistent",
        }
    }

    /// Parses the stored text; `None` for anything the schema's CHECK would have rejected.
    pub fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "pending" => Some(Self::Pending),
            "indexing" => Some(Self::Indexing),
            "persisting" => Some(Self::Persisting),
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            "inconsistent" => Some(Self::Inconsistent),
            _ => None,
        }
    }

    /// The only status `load_graph`, `neighbors` and `nodes` accept.
    pub const fn is_readable(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The only status `write_full`/`write_delta` accept.
    pub const fn is_writable(self) -> bool {
        matches!(self, Self::Persisting)
    }
}

impl std::fmt::Display for SnapshotStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The complete transition table. Everything not listed returns `false` from `transition`.
pub const ALLOWED_TRANSITIONS: &[(SnapshotStatus, SnapshotStatus)] = &[
    (SnapshotStatus::Pending, SnapshotStatus::Indexing),
    (SnapshotStatus::Pending, SnapshotStatus::Failed),
    (SnapshotStatus::Indexing, SnapshotStatus::Persisting),
    (SnapshotStatus::Indexing, SnapshotStatus::Failed),
    (SnapshotStatus::Persisting, SnapshotStatus::Ready),
    (SnapshotStatus::Persisting, SnapshotStatus::Failed),
    (SnapshotStatus::Ready, SnapshotStatus::Inconsistent),
];

/// Whether `from -> to` exists in the state machine. Self-transitions are never allowed.
pub fn can_transition(from: SnapshotStatus, to: SnapshotStatus) -> bool {
    ALLOWED_TRANSITIONS.contains(&(from, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_text_roundtrips() {
        for status in SnapshotStatus::ALL {
            assert_eq!(SnapshotStatus::from_db(status.as_str()), Some(status));
            assert_eq!(status.to_string(), status.as_str());
        }
        assert_eq!(SnapshotStatus::from_db("building"), None);
        assert_eq!(SnapshotStatus::ALL.len(), 6);
    }

    #[test]
    fn allowed_edges_match_the_diagram() {
        for (from, to) in ALLOWED_TRANSITIONS {
            assert!(can_transition(*from, *to), "{from} -> {to}");
        }
        assert!(can_transition(
            SnapshotStatus::Pending,
            SnapshotStatus::Indexing
        ));
        assert!(can_transition(
            SnapshotStatus::Indexing,
            SnapshotStatus::Persisting
        ));
        assert!(can_transition(
            SnapshotStatus::Persisting,
            SnapshotStatus::Ready
        ));
        assert!(can_transition(
            SnapshotStatus::Persisting,
            SnapshotStatus::Failed
        ));
        assert!(can_transition(
            SnapshotStatus::Ready,
            SnapshotStatus::Inconsistent
        ));
    }

    #[test]
    fn illegal_edges_are_rejected() {
        for (from, to) in [
            (SnapshotStatus::Pending, SnapshotStatus::Ready),
            (SnapshotStatus::Pending, SnapshotStatus::Persisting),
            (SnapshotStatus::Ready, SnapshotStatus::Indexing),
            (SnapshotStatus::Ready, SnapshotStatus::Failed),
            (SnapshotStatus::Failed, SnapshotStatus::Pending),
            (SnapshotStatus::Inconsistent, SnapshotStatus::Ready),
            (SnapshotStatus::Ready, SnapshotStatus::Ready),
        ] {
            assert!(!can_transition(from, to), "{from} -> {to} must be illegal");
        }
        let mut pairs = 0;
        for from in SnapshotStatus::ALL {
            for to in SnapshotStatus::ALL {
                if can_transition(from, to) {
                    pairs += 1;
                }
            }
        }
        assert_eq!(pairs, ALLOWED_TRANSITIONS.len());
    }

    #[test]
    fn only_ready_is_readable_and_only_persisting_is_writable() {
        for status in SnapshotStatus::ALL {
            assert_eq!(status.is_readable(), status == SnapshotStatus::Ready);
            assert_eq!(status.is_writable(), status == SnapshotStatus::Persisting);
        }
    }
}
