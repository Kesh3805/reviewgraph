//! Feedback rate formulas (PRD sections 116 and 117), computed over a trailing 7-day window.
//!
//! The definitions are written in `docs/operations/metrics-catalog.md` and must stay in step
//! with the evaluation tasks (EVAL-*).

/// Counts of published findings with feedback in the trailing window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedbackWindow {
    pub useful: u64,
    pub already_handled: u64,
    pub false_positive: u64,
    /// Published findings that received any feedback.
    pub published_with_feedback: u64,
}

/// Trailing window length in days.
pub const WINDOW_DAYS: u32 = 7;

impl FeedbackWindow {
    /// `(useful + already_handled) / published_with_feedback`, `None` without feedback.
    pub fn acceptance_rate(&self) -> Option<f64> {
        ratio(
            self.useful + self.already_handled,
            self.published_with_feedback,
        )
    }

    /// `false_positive / published_with_feedback`, `None` without feedback.
    pub fn false_positive_rate(&self) -> Option<f64> {
        ratio(self.false_positive, self.published_with_feedback)
    }
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    Some(numerator as f64 / denominator as f64)
}
