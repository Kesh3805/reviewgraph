//! `ReviewerRun`: one reviewer execution inside a review run, with model accounting (ADR-009).
//! Maps onto `reviewer_runs`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::change::ChangeClusterKey;
use crate::error::{CoreError, ErrorClass};
use crate::ids::{OrganizationId, ReviewRunId, ReviewerRunId};
use crate::reviewer_type::ReviewerType;
use crate::version::{PromptVersion, ReviewerVersion};

/// Wire form is snake_case. `Pending` goes to `Running` or `Skipped`; `Running` goes to
/// `Succeeded`, `Failed` or `TimedOut`; everything else is terminal.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerRunState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
    TimedOut,
}

impl ReviewerRunState {
    pub const ALL: [ReviewerRunState; 6] = [
        Self::Pending,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Skipped,
        Self::TimedOut,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::TimedOut => "timed_out",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Skipped | Self::TimedOut
        )
    }

    /// States that must carry an `error_class`.
    pub const fn is_error(self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut)
    }

    pub const fn can_transition_to(self, to: ReviewerRunState) -> bool {
        matches!(
            (self, to),
            (Self::Pending, Self::Running | Self::Skipped)
                | (
                    Self::Running,
                    Self::Succeeded | Self::Failed | Self::TimedOut
                )
        )
    }
}

/// Token accounting for one reviewer run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cached_read: u64,
    pub cached_write: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewerRun {
    pub id: ReviewerRunId,
    pub organization_id: OrganizationId,
    pub review_run_id: ReviewRunId,
    pub reviewer: ReviewerType,
    /// Unique together with `(review_run_id, reviewer)`, so a retried stage maps to the same row.
    pub cluster_key: Option<ChangeClusterKey>,
    pub state: ReviewerRunState,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prompt_version: Option<PromptVersion>,
    pub reviewer_version: Option<ReviewerVersion>,
    pub usage: TokenUsage,
    /// Cost in millionths of a US dollar.
    pub cost_usd_micros: u64,
    pub latency_ms: Option<u32>,
    /// Present exactly when the state is `Failed` or `TimedOut`.
    pub error_class: Option<ErrorClass>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl ReviewerRun {
    /// A new run in `Pending` with empty accounting.
    pub fn new(
        id: ReviewerRunId,
        organization_id: OrganizationId,
        review_run_id: ReviewRunId,
        reviewer: ReviewerType,
        cluster_key: Option<ChangeClusterKey>,
    ) -> Self {
        Self {
            id,
            organization_id,
            review_run_id,
            reviewer,
            cluster_key,
            state: ReviewerRunState::Pending,
            provider: None,
            model: None,
            prompt_version: None,
            reviewer_version: None,
            usage: TokenUsage::default(),
            cost_usd_micros: 0,
            latency_ms: None,
            error_class: None,
            started_at: None,
            finished_at: None,
        }
    }

    /// `error_class` must be present exactly when the state is `Failed` or `TimedOut`.
    pub fn validated(self) -> Result<Self, CoreError> {
        if self.error_class.is_some() != self.state.is_error() {
            return Err(CoreError::InvalidTransition {
                entity: "ReviewerRun",
                from: self.state.as_str(),
                to: self.state.as_str(),
            });
        }
        Ok(self)
    }

    /// Moves along an edge. `error_class` must be `Some` exactly for `Failed` and `TimedOut`.
    /// Sets `started_at` on `Running` and `finished_at` on every terminal state.
    pub fn transition(
        &mut self,
        to: ReviewerRunState,
        error_class: Option<ErrorClass>,
        at: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let invalid = || CoreError::InvalidTransition {
            entity: "ReviewerRun",
            from: self.state.as_str(),
            to: to.as_str(),
        };
        if !self.state.can_transition_to(to) || error_class.is_some() != to.is_error() {
            return Err(invalid());
        }
        self.state = to;
        self.error_class = error_class;
        if to == ReviewerRunState::Running {
            self.started_at = Some(at);
        }
        if to.is_terminal() {
            self.finished_at = Some(at);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ReviewerRun {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            id: ReviewerRunId,
            organization_id: OrganizationId,
            review_run_id: ReviewRunId,
            reviewer: ReviewerType,
            cluster_key: Option<ChangeClusterKey>,
            state: ReviewerRunState,
            provider: Option<String>,
            model: Option<String>,
            prompt_version: Option<PromptVersion>,
            reviewer_version: Option<ReviewerVersion>,
            usage: TokenUsage,
            cost_usd_micros: u64,
            latency_ms: Option<u32>,
            error_class: Option<ErrorClass>,
            started_at: Option<DateTime<Utc>>,
            finished_at: Option<DateTime<Utc>>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            id: r.id,
            organization_id: r.organization_id,
            review_run_id: r.review_run_id,
            reviewer: r.reviewer,
            cluster_key: r.cluster_key,
            state: r.state,
            provider: r.provider,
            model: r.model,
            prompt_version: r.prompt_version,
            reviewer_version: r.reviewer_version,
            usage: r.usage,
            cost_usd_micros: r.cost_usd_micros,
            latency_ms: r.latency_ms,
            error_class: r.error_class,
            started_at: r.started_at,
            finished_at: r.finished_at,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn run() -> ReviewerRun {
        ReviewerRun::new(
            ReviewerRunId::new(),
            OrganizationId::new(),
            ReviewRunId::new(),
            ReviewerType::Security,
            None,
        )
    }

    #[test]
    fn reviewer_run_state_edges() {
        use ReviewerRunState::*;
        let allowed = [
            (Pending, Running),
            (Pending, Skipped),
            (Running, Succeeded),
            (Running, Failed),
            (Running, TimedOut),
        ];
        for from in ReviewerRunState::ALL {
            for to in ReviewerRunState::ALL {
                assert_eq!(
                    from.can_transition_to(to),
                    allowed.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
            }
        }
        for s in [Succeeded, Failed, Skipped, TimedOut] {
            assert!(s.is_terminal());
        }
    }

    #[test]
    fn reviewer_run_error_class_iff_failed_or_timed_out() {
        for terminal in [ReviewerRunState::Failed, ReviewerRunState::TimedOut] {
            let mut r = run();
            r.transition(ReviewerRunState::Running, None, at()).unwrap();
            assert!(r.transition(terminal, None, at()).is_err());
            assert_eq!(r.state, ReviewerRunState::Running);
            r.transition(terminal, Some(ErrorClass::Transient), at())
                .unwrap();
            assert_eq!(r.error_class, Some(ErrorClass::Transient));
            assert!(r.finished_at.is_some());
        }
        {
            let terminal = ReviewerRunState::Succeeded;
            let mut r = run();
            r.transition(ReviewerRunState::Running, None, at()).unwrap();
            assert!(r
                .transition(terminal, Some(ErrorClass::Internal), at())
                .is_err());
            r.transition(terminal, None, at()).unwrap();
            assert_eq!(r.error_class, None);
        }
        let mut skipped = run();
        assert!(skipped
            .transition(ReviewerRunState::Skipped, Some(ErrorClass::Internal), at())
            .is_err());
        skipped
            .transition(ReviewerRunState::Skipped, None, at())
            .unwrap();
        assert!(skipped.finished_at.is_some() && skipped.started_at.is_none());
        // Terminal states do not move.
        assert!(skipped
            .transition(ReviewerRunState::Running, None, at())
            .is_err());
    }

    #[test]
    fn running_sets_started_at() {
        let mut r = run();
        r.transition(ReviewerRunState::Running, None, at()).unwrap();
        assert_eq!(r.started_at, Some(at()));
        assert_eq!(r.finished_at, None);
    }

    #[test]
    fn deserialize_enforces_error_class_invariant() {
        let mut r = run();
        r.transition(ReviewerRunState::Running, None, at()).unwrap();
        r.transition(ReviewerRunState::Failed, Some(ErrorClass::Permanent), at())
            .unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ReviewerRun>(&json).unwrap(), r);
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["error_class"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<ReviewerRun>(v).is_err());
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["state"] = serde_json::json!("timed_out");
        assert!(serde_json::from_value::<ReviewerRun>(v).is_ok());
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["state"] = serde_json::json!("succeeded");
        assert!(serde_json::from_value::<ReviewerRun>(v).is_err());
    }

    #[test]
    fn wire_names() {
        assert_eq!(
            serde_json::to_string(&ReviewerRunState::TimedOut).unwrap(),
            "\"timed_out\""
        );
        for s in ReviewerRunState::ALL {
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
    }
}
