//! Relevance ranking (CTX-005): ten signals in `[0, 1]`, per-reviewer weights and the distance gate.

pub mod gate;
pub mod weights;

pub use gate::{gate, GateOutcome};
pub use weights::{WeightError, WeightTable, Weights, MAX_WEIGHT, WEIGHTS_VERSION};

use serde::{Deserialize, Serialize};

/// The ten relevance signals, each clamped to `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Signals {
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

impl Signals {
    /// Signals other than proximity, in a fixed order.
    pub fn non_proximity(&self) -> [f32; 9] {
        [
            self.structural,
            self.execution_path,
            self.changed_code_similarity,
            self.test_relationship,
            self.api_relationship,
            self.config_relationship,
            self.semantic_similarity,
            self.historical,
            self.risk_importance,
        ]
    }

    pub fn clamped(self) -> Self {
        let c = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        Self {
            structural: c(self.structural),
            proximity: c(self.proximity),
            execution_path: c(self.execution_path),
            changed_code_similarity: c(self.changed_code_similarity),
            test_relationship: c(self.test_relationship),
            api_relationship: c(self.api_relationship),
            config_relationship: c(self.config_relationship),
            semantic_similarity: c(self.semantic_similarity),
            historical: c(self.historical),
            risk_importance: c(self.risk_importance),
        }
    }
}

/// PRD section 34 proximity heuristic.
pub fn proximity(distance: Option<u8>) -> f32 {
    match distance {
        Some(0) => 1.0,
        Some(1) => 0.8,
        Some(2) => 0.5,
        Some(3) | None => 0.2,
        Some(_) => 0.0,
    }
}

/// Relation strength for the `structural` signal.
pub fn structural_strength(relation: &str) -> f32 {
    match relation {
        "caller" | "callee" | "removed_callee" => 1.0,
        "interface" | "implementation" | "override" => 0.9,
        "related_type" | "resource" => 0.6,
        "container" => 0.5,
        _ => 0.0,
    }
}

/// Maps a cosine similarity `s` to `(s - 0.5) / 0.5` clamped to `[0, 1]`.
pub fn semantic_signal(cosine: f32) -> f32 {
    ((cosine - 0.5) / 0.5).clamp(0.0, 1.0)
}

fn round6(v: f32) -> f32 {
    (v * 1_000_000.0).round() / 1_000_000.0
}

/// `score = sum(w_i * s_i) * conf_penalty`, `conf_penalty = 0.5 + 0.5 * min_confidence`,
/// weak edges an extra x0.7. Rounded to 6 decimals so comparisons are stable.
pub fn score(signals: &Signals, weights: &Weights, min_confidence: f32, weak: bool) -> f32 {
    let s = signals.clamped();
    let w = weights;
    let sum = w.structural * s.structural
        + w.proximity * s.proximity
        + w.execution_path * s.execution_path
        + w.changed_code_similarity * s.changed_code_similarity
        + w.test_relationship * s.test_relationship
        + w.api_relationship * s.api_relationship
        + w.config_relationship * s.config_relationship
        + w.semantic_similarity * s.semantic_similarity
        + w.historical * s.historical
        + w.risk_importance * s.risk_importance;
    let conf = if min_confidence.is_nan() {
        0.0
    } else {
        min_confidence.clamp(0.0, 1.0)
    };
    let mut out = sum * (0.5 + 0.5 * conf);
    if weak {
        out *= 0.7;
    }
    round6(out)
}
