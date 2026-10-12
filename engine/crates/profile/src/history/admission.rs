//! The `SignalAdmission` gate (HIST-004, PRD §69).
//!
//! One bad review must never become a stored policy that repeats forever. A history-derived
//! signal, convention or suppression influences a review only after it passes this gate:
//!
//! 1. repetition: at least 5 independent observations over at least 3 distinct pull requests and
//!    2 distinct hashed reviewers (single-source guard: ReviewGraph's own findings are not
//!    independent evidence);
//! 2. repository evidence: the pattern holds in at least 90% of at least 10 samples;
//! 3. explicit acceptance (`useful` feedback, a maintainer label, an inclusion in
//!    `.review/config.yaml`) or a human-configured policy.
//!
//! A signal with only (1) and (2) may influence ranking at a reduced weight and can never
//! suppress or create a finding. Evidence that contradicts the signal demotes it. Missing or
//! failing evidence fails closed. `history.enabled = false` rejects everything.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

pub const MIN_OBSERVATIONS: u32 = 5;
pub const MIN_DISTINCT_PRS: u32 = 3;
pub const MIN_DISTINCT_REVIEWERS: u32 = 2;
/// Repository corroboration: the pattern must hold in this share of at least [`MIN_SAMPLES`].
pub const MIN_HOLDING_RATIO: f64 = 0.90;
pub const MIN_SAMPLES: u32 = 10;
/// Below this share of at least [`MIN_SAMPLES`] the repository contradicts the signal.
pub const CONTRADICTION_RATIO: f64 = 0.50;
/// Ranking weight of a signal that lacks acceptance.
pub const RANKING_ONLY_MAX_WEIGHT: f64 = 0.05;
/// Ranking weight of a fully admitted signal.
pub const ADMITTED_MAX_WEIGHT: f64 = 0.15;
/// Pattern-level suppression expires unless re-confirmed.
pub const PATTERN_SUPPRESSION_TTL_DAYS: i64 = 90;

/// Where an observation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A human reviewer or an external tool.
    Independent,
    /// One of ReviewGraph's own past findings: never independent evidence.
    ReviewGraphOwn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub pr_id: String,
    /// Salted hash of the reviewer behind the observation.
    pub reviewer_hash: String,
    pub origin: Origin,
}

/// How well the repository itself supports the pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corroboration {
    pub samples: u32,
    pub holding: u32,
}

impl Corroboration {
    fn ratio(self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            f64::from(self.holding) / f64::from(self.samples)
        }
    }
}

/// Human acceptance of a signal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Acceptance {
    pub explicit_useful_feedback: bool,
    pub maintainer_label: bool,
    pub in_repository_config: bool,
    pub human_policy: bool,
}

impl Acceptance {
    fn any(self) -> bool {
        self.explicit_useful_feedback
            || self.maintainer_label
            || self.in_repository_config
            || self.human_policy
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdmissionInput {
    /// Stable id of the signal (`kind:scope`).
    pub signal_id: String,
    pub observations: Vec<Observation>,
    pub corroboration: Option<Corroboration>,
    pub acceptance: Acceptance,
    /// `history.enabled` of the repository (the kill switch).
    pub history_enabled: bool,
    /// Whether the signal wants to suppress findings (pattern-level suppression).
    pub wants_suppression: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    KillSwitch,
    /// Evidence was missing or evaluation failed.
    FailedClosed,
    TooFewObservations,
    TooFewDistinctPrs,
    TooFewDistinctReviewers,
    NoRepositoryEvidence,
    ContradictedByRepository,
    NotAccepted,
    /// Admitted for ranking only, at reduced weight.
    RankingOnly,
    Admitted,
}

impl Reason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KillSwitch => "kill_switch",
            Self::FailedClosed => "failed_closed",
            Self::TooFewObservations => "too_few_observations",
            Self::TooFewDistinctPrs => "too_few_distinct_prs",
            Self::TooFewDistinctReviewers => "too_few_distinct_reviewers",
            Self::NoRepositoryEvidence => "no_repository_evidence",
            Self::ContradictedByRepository => "contradicted_by_repository",
            Self::NotAccepted => "not_accepted",
            Self::RankingOnly => "ranking_only",
            Self::Admitted => "admitted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionOutcome {
    Admitted,
    Rejected,
    Demoted,
}

impl AdmissionOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::Rejected => "rejected",
            Self::Demoted => "demoted",
        }
    }
}

/// What a signal may do after the gate.
#[derive(Debug, Clone, PartialEq)]
pub struct AdmissionDecision {
    pub signal_id: String,
    pub outcome: AdmissionOutcome,
    pub reasons: Vec<Reason>,
    /// Upper bound of the ranking weight; 0 when rejected.
    pub max_weight: f64,
    /// Only a fully admitted signal may suppress, and only at pattern level with an expiry.
    pub may_suppress: bool,
    /// A signal never creates findings; kept explicit so callers cannot assume otherwise.
    pub may_create_findings: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub evidence_summary: Value,
    /// Same inputs, same hash: re-evaluation does not duplicate audit rows.
    pub input_hash: String,
    pub decided_at: DateTime<Utc>,
}

impl AdmissionDecision {
    /// Counts and ids only; this is what `review history explain <signal_id>` prints.
    pub fn explain(&self) -> String {
        let reasons: Vec<&str> = self.reasons.iter().map(|r| r.as_str()).collect();
        format!(
            "signal {}: {} (reasons: {}; max weight {:.2}; may suppress: {}; evidence: {})",
            self.signal_id,
            self.outcome.as_str(),
            reasons.join(", "),
            self.max_weight,
            self.may_suppress,
            self.evidence_summary
        )
    }

    pub fn reasons_json(&self) -> Value {
        json!(self.reasons.iter().map(|r| r.as_str()).collect::<Vec<_>>())
    }
}

fn input_hash(input: &AdmissionInput) -> String {
    let independent: BTreeSet<(&str, &str)> = input
        .observations
        .iter()
        .filter(|o| o.origin == Origin::Independent)
        .map(|o| (o.pr_id.as_str(), o.reviewer_hash.as_str()))
        .collect();
    let own = input
        .observations
        .iter()
        .filter(|o| o.origin == Origin::ReviewGraphOwn)
        .count();
    let text = format!(
        "{}|{independent:?}|{own}|{:?}|{:?}|{}|{}",
        input.signal_id,
        input.corroboration.map(|c| (c.samples, c.holding)),
        input.acceptance,
        input.history_enabled,
        input.wants_suppression
    );
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

fn count(values: BTreeSet<&str>) -> u32 {
    u32::try_from(values.len()).unwrap_or(u32::MAX)
}

fn finish(
    input: &AdmissionInput,
    now: DateTime<Utc>,
    outcome: AdmissionOutcome,
    reasons: Vec<Reason>,
    max_weight: f64,
    may_suppress: bool,
    summary: Value,
) -> AdmissionDecision {
    AdmissionDecision {
        signal_id: input.signal_id.clone(),
        outcome,
        reasons,
        max_weight,
        may_suppress,
        may_create_findings: false,
        expires_at: may_suppress.then(|| now + Duration::days(PATTERN_SUPPRESSION_TTL_DAYS)),
        evidence_summary: summary,
        input_hash: input_hash(input),
        decided_at: now,
    }
}

/// Evaluates the gate. Pure: the same input and `now` give the same decision.
pub fn admit(input: &AdmissionInput, now: DateTime<Utc>) -> AdmissionDecision {
    let independent: Vec<&Observation> = input
        .observations
        .iter()
        .filter(|o| o.origin == Origin::Independent)
        .collect();
    let n = u32::try_from(independent.len()).unwrap_or(u32::MAX);
    let prs = count(independent.iter().map(|o| o.pr_id.as_str()).collect());
    let reviewers = count(
        independent
            .iter()
            .map(|o| o.reviewer_hash.as_str())
            .collect(),
    );
    let summary = json!({
        "independent_observations": n,
        "reviewgraph_own_observations": input.observations.len() - independent.len(),
        "distinct_prs": prs,
        "distinct_reviewers": reviewers,
        "samples": input.corroboration.map(|c| c.samples),
        "holding": input.corroboration.map(|c| c.holding),
    });

    if !input.history_enabled {
        return finish(
            input,
            now,
            AdmissionOutcome::Rejected,
            vec![Reason::KillSwitch],
            0.0,
            false,
            summary,
        );
    }

    // Contradicting repository evidence demotes a signal whatever else holds.
    if let Some(c) = input.corroboration {
        if c.samples >= MIN_SAMPLES && c.ratio() < CONTRADICTION_RATIO {
            return finish(
                input,
                now,
                AdmissionOutcome::Demoted,
                vec![Reason::ContradictedByRepository],
                0.0,
                false,
                summary,
            );
        }
    }

    let mut failures = Vec::new();
    if n < MIN_OBSERVATIONS {
        failures.push(Reason::TooFewObservations);
    }
    if prs < MIN_DISTINCT_PRS {
        failures.push(Reason::TooFewDistinctPrs);
    }
    if reviewers < MIN_DISTINCT_REVIEWERS {
        failures.push(Reason::TooFewDistinctReviewers);
    }
    let corroborated = input
        .corroboration
        .is_some_and(|c| c.samples >= MIN_SAMPLES && c.ratio() >= MIN_HOLDING_RATIO);
    if !corroborated {
        failures.push(Reason::NoRepositoryEvidence);
    }
    if !failures.is_empty() {
        return finish(
            input,
            now,
            AdmissionOutcome::Rejected,
            failures,
            0.0,
            false,
            summary,
        );
    }

    if input.acceptance.any() {
        finish(
            input,
            now,
            AdmissionOutcome::Admitted,
            vec![Reason::Admitted],
            ADMITTED_MAX_WEIGHT,
            input.wants_suppression,
            summary,
        )
    } else {
        finish(
            input,
            now,
            AdmissionOutcome::Admitted,
            vec![Reason::NotAccepted, Reason::RankingOnly],
            RANKING_ONLY_MAX_WEIGHT,
            false,
            summary,
        )
    }
}

/// Fails closed: evidence that could not be gathered rejects the signal and the review goes on
/// without it.
pub fn admit_or_reject(
    signal_id: &str,
    input: Result<AdmissionInput, String>,
    now: DateTime<Utc>,
) -> AdmissionDecision {
    match input {
        Ok(input) => admit(&input, now),
        Err(error) => {
            let hash = blake3::hash(format!("{signal_id}|failed|{error}").as_bytes());
            AdmissionDecision {
                signal_id: signal_id.to_owned(),
                outcome: AdmissionOutcome::Rejected,
                reasons: vec![Reason::FailedClosed],
                max_weight: 0.0,
                may_suppress: false,
                may_create_findings: false,
                expires_at: None,
                evidence_summary: json!({ "error": "evaluation_failed" }),
                input_hash: hash.to_hex().to_string(),
                decided_at: now,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0)
            .single()
            .unwrap_or_default()
    }

    fn obs(pr: &str, reviewer: &str, origin: Origin) -> Observation {
        Observation {
            pr_id: pr.to_owned(),
            reviewer_hash: reviewer.to_owned(),
            origin,
        }
    }

    /// 6 independent observations over 3 PRs and 2 reviewers.
    fn healthy() -> Vec<Observation> {
        (0..6)
            .map(|i| {
                obs(
                    &format!("pr{}", i % 3),
                    &format!("r{}", i % 2),
                    Origin::Independent,
                )
            })
            .collect()
    }

    fn input(observations: Vec<Observation>) -> AdmissionInput {
        AdmissionInput {
            signal_id: "dismissal_pattern:path:src/a".into(),
            observations,
            corroboration: Some(Corroboration {
                samples: 20,
                holding: 19,
            }),
            acceptance: Acceptance::default(),
            history_enabled: true,
            wants_suppression: true,
        }
    }

    #[test]
    fn single_review_never_creates_policy() {
        let one = input(vec![obs("pr1", "r1", Origin::Independent)]);
        let d = admit(&one, now());
        assert_eq!(d.outcome, AdmissionOutcome::Rejected);
        assert!(!d.may_suppress && !d.may_create_findings);
        assert_eq!(d.max_weight, 0.0);
    }

    #[test]
    fn single_author_repetition_is_insufficient() {
        // One reviewer rejecting the pattern 20 times in one pull request.
        let spam: Vec<Observation> = (0..20)
            .map(|_| obs("pr1", "r1", Origin::Independent))
            .collect();
        let mut i = input(spam);
        i.acceptance.explicit_useful_feedback = true;
        let d = admit(&i, now());
        assert_eq!(d.outcome, AdmissionOutcome::Rejected);
        assert!(d.reasons.contains(&Reason::TooFewDistinctPrs));
        assert!(d.reasons.contains(&Reason::TooFewDistinctReviewers));
        assert!(!d.may_suppress);
    }

    #[test]
    fn reviewgraph_own_findings_not_independent_evidence() {
        let own: Vec<Observation> = (0..10)
            .map(|i| {
                obs(
                    &format!("pr{i}"),
                    &format!("bot{}", i % 4),
                    Origin::ReviewGraphOwn,
                )
            })
            .collect();
        let mut i = input(own);
        i.acceptance.human_policy = true;
        let d = admit(&i, now());
        assert_eq!(d.outcome, AdmissionOutcome::Rejected);
        assert!(d.reasons.contains(&Reason::TooFewObservations));
        assert_eq!(d.evidence_summary["reviewgraph_own_observations"], 10);
    }

    #[test]
    fn unaccepted_signal_cannot_suppress_or_create_findings() {
        let d = admit(&input(healthy()), now());
        assert_eq!(d.outcome, AdmissionOutcome::Admitted);
        assert_eq!(d.max_weight, RANKING_ONLY_MAX_WEIGHT);
        assert!(!d.may_suppress, "no acceptance, no suppression");
        assert!(!d.may_create_findings);
        assert!(d.expires_at.is_none());
    }

    #[test]
    fn accepted_repeated_pattern_is_admitted_with_expiring_suppression() {
        let mut i = input(healthy());
        i.acceptance.maintainer_label = true;
        let d = admit(&i, now());
        assert_eq!(d.outcome, AdmissionOutcome::Admitted);
        assert!(d.may_suppress);
        assert!(!d.may_create_findings);
        assert_eq!(
            d.expires_at,
            Some(now() + Duration::days(PATTERN_SUPPRESSION_TTL_DAYS))
        );
    }

    #[test]
    fn pattern_suppression_expires_after_90_days() {
        let mut i = input(healthy());
        i.acceptance.in_repository_config = true;
        let d = admit(&i, now());
        let expires = d.expires_at.unwrap_or(now());
        assert_eq!((expires - now()).num_days(), 90);
    }

    #[test]
    fn contradicting_repository_evidence_demotes_signal() {
        let mut i = input(healthy());
        i.acceptance.human_policy = true;
        i.corroboration = Some(Corroboration {
            samples: 20,
            holding: 4,
        });
        let d = admit(&i, now());
        assert_eq!(d.outcome, AdmissionOutcome::Demoted);
        assert_eq!(d.max_weight, 0.0);
        assert!(!d.may_suppress);
    }

    #[test]
    fn missing_repository_evidence_rejects() {
        let mut i = input(healthy());
        i.corroboration = None;
        let d = admit(&i, now());
        assert_eq!(d.outcome, AdmissionOutcome::Rejected);
        assert!(d.reasons.contains(&Reason::NoRepositoryEvidence));
    }

    #[test]
    fn admission_fails_closed_on_error() {
        let d = admit_or_reject("sig", Err("graph unavailable".into()), now());
        assert_eq!(d.outcome, AdmissionOutcome::Rejected);
        assert_eq!(d.reasons, vec![Reason::FailedClosed]);
        assert_eq!(d.max_weight, 0.0);
        assert!(!d.may_suppress);
    }

    #[test]
    fn kill_switch_disables_all_history_influence() {
        let mut i = input(healthy());
        i.acceptance.human_policy = true;
        i.history_enabled = false;
        let d = admit(&i, now());
        assert_eq!(d.outcome, AdmissionOutcome::Rejected);
        assert_eq!(d.reasons, vec![Reason::KillSwitch]);
        assert_eq!(d.max_weight, 0.0);
    }

    #[test]
    fn same_input_same_hash_and_explanation_has_counts_only() {
        let a = admit(&input(healthy()), now());
        let b = admit(&input(healthy()), now());
        assert_eq!(a.input_hash, b.input_hash);
        let mut changed = input(healthy());
        changed.acceptance.maintainer_label = true;
        assert_ne!(a.input_hash, admit(&changed, now()).input_hash);
        let text = a.explain();
        assert!(text.contains("distinct_prs"));
        assert!(!text.contains("r0"), "reviewer hashes are not printed");
    }

    /// Adversarial history: one reviewer rejecting a pattern 20 times in one pull request plus
    /// ReviewGraph's findings repeated 10 times admits zero suppressing signals.
    #[test]
    fn adversarial_history_admits_no_suppression() {
        let mut observations: Vec<Observation> = (0..20)
            .map(|_| obs("pr1", "r1", Origin::Independent))
            .collect();
        observations.extend((0..10).map(|i| obs(&format!("pr{i}"), "bot", Origin::ReviewGraphOwn)));
        let mut i = input(observations);
        i.acceptance.explicit_useful_feedback = true;
        let d = admit(&i, now());
        assert!(!d.may_suppress);
        assert_eq!(d.outcome, AdmissionOutcome::Rejected);
    }
}
