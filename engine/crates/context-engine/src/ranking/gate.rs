//! Distance gate (PRD section 34).

use super::Signals;
use crate::model::{ItemKind, OmitReason};

#[derive(Debug, Clone, PartialEq)]
pub enum GateOutcome {
    Pass,
    Reject(OmitReason),
}

/// d0 always passes; d1 needs score >= 0.20; d2 >= 0.35; d3 needs score >= 0.50 and at least two
/// non-proximity signals >= 0.5; d4+ is excluded unless it lies on an endpoint path
/// (`execution_path == 1.0`) or is a Test/ApiEndpoint item. No distance counts as d3.
pub fn gate(distance: Option<u8>, score: f32, signals: &Signals, kind: ItemKind) -> GateOutcome {
    let reject_distance = |d: u8| GateOutcome::Reject(OmitReason::DistanceRule { distance: d });
    let below =
        |threshold: f32| GateOutcome::Reject(OmitReason::BelowThreshold { score, threshold });
    match distance {
        Some(0) => GateOutcome::Pass,
        Some(1) => {
            if score >= 0.20 {
                GateOutcome::Pass
            } else {
                below(0.20)
            }
        }
        Some(2) => {
            if score >= 0.35 {
                GateOutcome::Pass
            } else {
                below(0.35)
            }
        }
        Some(d) if d >= 4 => {
            if signals.execution_path >= 1.0
                || matches!(kind, ItemKind::Test | ItemKind::ApiEndpoint)
            {
                GateOutcome::Pass
            } else {
                reject_distance(d)
            }
        }
        // d3 and candidates without a graph distance.
        _ => {
            let strong = signals
                .non_proximity()
                .iter()
                .filter(|v| **v >= 0.5)
                .count();
            if score >= 0.50 && strong >= 2 {
                GateOutcome::Pass
            } else if score < 0.50 {
                below(0.50)
            } else {
                reject_distance(distance.unwrap_or(3))
            }
        }
    }
}
