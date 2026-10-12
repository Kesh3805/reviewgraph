//! Finding outcome derivation (HIST-002, PRD §67).
//!
//! Every published finding has an outcome with a recorded source. Precedence is fixed:
//! explicit feedback (and suppression creation) > code-change evidence > thread state >
//! merge-with-no-action. Events are append-only; the current outcome is derived from them, so a
//! concurrent writer can never lose an explicit verdict.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Fix applied, or explicitly marked useful.
    Accepted,
    /// `false_positive` or `not_relevant`.
    Rejected,
    /// `intentional`, a suppression was created, or a human dismissed the thread.
    Dismissed,
    /// The thread was resolved with no code change in the cited range.
    ResolvedUnattributed,
    /// The pull request ended with the thread untouched and no feedback.
    Ignored,
    Pending,
}

impl Outcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Dismissed => "dismissed",
            Self::ResolvedUnattributed => "resolved_unattributed",
            Self::Ignored => "ignored",
            Self::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutcomeSource {
    ExplicitFeedback,
    Suppression,
    CodeChange,
    ThreadState,
    MergeNoAction,
}

impl OutcomeSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitFeedback => "explicit_feedback",
            Self::Suppression => "suppression",
            Self::CodeChange => "code_change",
            Self::ThreadState => "thread_state",
            Self::MergeNoAction => "merge_no_action",
        }
    }

    /// Higher wins.
    pub const fn precedence(self) -> u8 {
        match self {
            Self::ExplicitFeedback | Self::Suppression => 4,
            Self::CodeChange => 3,
            Self::ThreadState => 2,
            Self::MergeNoAction => 1,
        }
    }

    pub const fn is_explicit(self) -> bool {
        matches!(self, Self::ExplicitFeedback | Self::Suppression)
    }
}

/// Feedback verdicts (API-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Useful,
    FalsePositive,
    AlreadyHandled,
    NotRelevant,
    Intentional,
}

impl Verdict {
    pub const fn outcome(self) -> Outcome {
        match self {
            Self::Useful | Self::AlreadyHandled => Outcome::Accepted,
            Self::FalsePositive | Self::NotRelevant => Outcome::Rejected,
            Self::Intentional => Outcome::Dismissed,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "useful" => Some(Self::Useful),
            "false_positive" => Some(Self::FalsePositive),
            "already_handled" => Some(Self::AlreadyHandled),
            "not_relevant" => Some(Self::NotRelevant),
            "intentional" => Some(Self::Intentional),
            _ => None,
        }
    }
}

/// One appended outcome event.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeEvent {
    pub outcome: Outcome,
    pub source: OutcomeSource,
    /// Ids and hashes only (commit sha, thread id): never comment bodies.
    pub evidence: Value,
    pub inferred: bool,
    /// 1.0 for explicit events, below 1.0 for inferred ones.
    pub confidence: f32,
    pub created_at: DateTime<Utc>,
}

/// What a commit pushed after publication touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitTouch {
    pub sha: String,
    /// Symbols whose bodies the commit changed (from the incremental diff).
    pub touched_symbols: Vec<String>,
    /// `(path, start_line, end_line)` of the changed hunks.
    pub touched_ranges: Vec<(String, i32, i32)>,
    pub committed_at: DateTime<Utc>,
}

/// Everything known about one published finding.
#[derive(Debug, Clone, PartialEq)]
pub struct FindingFacts {
    pub path: Option<String>,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub symbol_key: Option<String>,
    pub published_at: DateTime<Utc>,
    pub thread_id: Option<String>,
    pub thread_resolved: bool,
    pub thread_dismissed_by_human: bool,
    pub pr_closed_or_merged: bool,
    pub feedback: Vec<(Verdict, DateTime<Utc>)>,
    pub suppression_created_at: Option<DateTime<Utc>>,
    pub commits_after_publish: Vec<CommitTouch>,
    pub now: DateTime<Utc>,
}

/// The commit (if any) whose diff touches the cited symbol, or else the cited range.
fn fix_commit(facts: &FindingFacts) -> Option<(&CommitTouch, bool)> {
    let symbol_hit = facts.symbol_key.as_deref().and_then(|symbol| {
        facts
            .commits_after_publish
            .iter()
            .filter(|c| c.committed_at >= facts.published_at)
            .find(|c| c.touched_symbols.iter().any(|s| s == symbol))
    });
    if let Some(commit) = symbol_hit {
        return Some((commit, true));
    }
    let (path, start, end) = (
        facts.path.as_deref()?,
        facts.start_line?,
        facts.end_line.or(facts.start_line)?,
    );
    facts
        .commits_after_publish
        .iter()
        .filter(|c| c.committed_at >= facts.published_at)
        .find(|c| {
            c.touched_ranges
                .iter()
                .any(|(p, s, e)| p == path && *s <= end && start <= *e)
        })
        .map(|c| (c, false))
}

/// The events a snapshot of `facts` justifies (all of them, not only the winner).
pub fn infer_events(facts: &FindingFacts) -> Vec<OutcomeEvent> {
    let mut events = Vec::new();
    for (verdict, at) in &facts.feedback {
        events.push(OutcomeEvent {
            outcome: verdict.outcome(),
            source: OutcomeSource::ExplicitFeedback,
            evidence: json!({ "verdict": format!("{verdict:?}") }),
            inferred: false,
            confidence: 1.0,
            created_at: *at,
        });
    }
    if let Some(at) = facts.suppression_created_at {
        events.push(OutcomeEvent {
            outcome: Outcome::Dismissed,
            source: OutcomeSource::Suppression,
            evidence: json!({}),
            inferred: false,
            confidence: 1.0,
            created_at: at,
        });
    }
    let fix = fix_commit(facts);
    if let Some((commit, symbol_level)) = fix {
        events.push(OutcomeEvent {
            outcome: Outcome::Accepted,
            source: OutcomeSource::CodeChange,
            evidence: json!({ "commit_sha": commit.sha, "symbol_level": symbol_level }),
            inferred: true,
            confidence: if symbol_level { 0.9 } else { 0.7 },
            created_at: commit.committed_at,
        });
    }
    if facts.thread_dismissed_by_human {
        events.push(OutcomeEvent {
            outcome: Outcome::Dismissed,
            source: OutcomeSource::ThreadState,
            evidence: json!({ "thread_id": facts.thread_id, "state": "dismissed" }),
            inferred: true,
            confidence: 0.8,
            created_at: facts.now,
        });
    } else if facts.thread_resolved && fix.is_none() {
        events.push(OutcomeEvent {
            outcome: Outcome::ResolvedUnattributed,
            source: OutcomeSource::ThreadState,
            evidence: json!({ "thread_id": facts.thread_id, "state": "resolved" }),
            inferred: true,
            confidence: 0.6,
            created_at: facts.now,
        });
    }
    if facts.pr_closed_or_merged
        && !facts.thread_resolved
        && !facts.thread_dismissed_by_human
        && fix.is_none()
        && facts.feedback.is_empty()
        && facts.suppression_created_at.is_none()
    {
        events.push(OutcomeEvent {
            outcome: Outcome::Ignored,
            source: OutcomeSource::MergeNoAction,
            evidence: json!({ "thread_id": facts.thread_id }),
            inferred: true,
            confidence: 0.5,
            created_at: facts.now,
        });
    }
    events
}

/// The current outcome: highest precedence, then the latest event.
pub fn current_outcome(events: &[OutcomeEvent]) -> Option<&OutcomeEvent> {
    events
        .iter()
        .max_by_key(|e| (e.source.precedence(), e.created_at))
}

/// The current outcome of `facts`; `Pending` when nothing is known yet.
pub fn derive_outcome(facts: &FindingFacts) -> Outcome {
    let events = infer_events(facts);
    current_outcome(&events).map_or(Outcome::Pending, |e| e.outcome)
}

/// Dedup key of an event: `(finding, source, evidence_hash)` is unique, so replaying a webhook
/// adds nothing.
pub fn evidence_hash(event: &OutcomeEvent) -> String {
    let text = format!(
        "{}\u{0}{}\u{0}{}",
        event.source.as_str(),
        event.outcome.as_str(),
        event.evidence
    );
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn t(minutes: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0)
            .single()
            .unwrap_or_default()
            + Duration::minutes(minutes)
    }

    fn facts() -> FindingFacts {
        FindingFacts {
            path: Some("src/a.ts".into()),
            start_line: Some(10),
            end_line: Some(12),
            symbol_key: Some("a.run".into()),
            published_at: t(0),
            thread_id: Some("T1".into()),
            thread_resolved: false,
            thread_dismissed_by_human: false,
            pr_closed_or_merged: false,
            feedback: Vec::new(),
            suppression_created_at: None,
            commits_after_publish: Vec::new(),
            now: t(100),
        }
    }

    fn touch(symbols: &[&str], ranges: &[(&str, i32, i32)]) -> CommitTouch {
        CommitTouch {
            sha: "c".repeat(40),
            touched_symbols: symbols.iter().map(|s| (*s).to_owned()).collect(),
            touched_ranges: ranges
                .iter()
                .map(|(p, s, e)| ((*p).to_owned(), *s, *e))
                .collect(),
            committed_at: t(10),
        }
    }

    #[test]
    fn nothing_known_is_pending() {
        assert_eq!(derive_outcome(&facts()), Outcome::Pending);
    }

    #[test]
    fn explicit_feedback_overrides_inferred_outcome() {
        let mut f = facts();
        f.commits_after_publish = vec![touch(&["a.run"], &[])];
        f.feedback = vec![(Verdict::FalsePositive, t(20))];
        assert_eq!(derive_outcome(&f), Outcome::Rejected);
    }

    #[test]
    fn code_change_in_cited_symbol_marks_accepted() {
        let mut f = facts();
        // The line numbers moved, the symbol is what matches.
        f.commits_after_publish = vec![touch(&["a.run"], &[("src/other.ts", 1, 2)])];
        let events = infer_events(&f);
        let winner = current_outcome(&events).map(|e| (e.outcome, e.source, e.inferred));
        assert_eq!(
            winner,
            Some((Outcome::Accepted, OutcomeSource::CodeChange, true))
        );
    }

    #[test]
    fn range_overlap_is_a_weaker_code_change() {
        let mut f = facts();
        f.symbol_key = None;
        f.commits_after_publish = vec![touch(&[], &[("src/a.ts", 12, 20)])];
        let events = infer_events(&f);
        assert_eq!(events.len(), 1);
        assert!(events[0].confidence < 0.9 && events[0].confidence < 1.0);
    }

    #[test]
    fn merge_without_action_marks_ignored() {
        let mut f = facts();
        f.pr_closed_or_merged = true;
        assert_eq!(derive_outcome(&f), Outcome::Ignored);
    }

    #[test]
    fn resolved_without_code_change_is_unattributed() {
        let mut f = facts();
        f.thread_resolved = true;
        f.pr_closed_or_merged = true;
        assert_eq!(derive_outcome(&f), Outcome::ResolvedUnattributed);
    }

    #[test]
    fn suppression_dismisses() {
        let mut f = facts();
        f.suppression_created_at = Some(t(30));
        f.pr_closed_or_merged = true;
        assert_eq!(derive_outcome(&f), Outcome::Dismissed);
    }

    #[test]
    fn latest_explicit_verdict_wins() {
        let mut f = facts();
        f.feedback = vec![(Verdict::FalsePositive, t(5)), (Verdict::Useful, t(9))];
        assert_eq!(derive_outcome(&f), Outcome::Accepted);
    }

    #[test]
    fn commits_before_publication_are_not_fixes() {
        let mut f = facts();
        let mut early = touch(&["a.run"], &[]);
        early.committed_at = t(-5);
        f.commits_after_publish = vec![early];
        assert_eq!(derive_outcome(&f), Outcome::Pending);
    }

    #[test]
    fn evidence_hash_is_stable_and_distinguishes_evidence() {
        let mut f = facts();
        f.commits_after_publish = vec![touch(&["a.run"], &[])];
        let a = infer_events(&f);
        let b = infer_events(&f);
        assert_eq!(evidence_hash(&a[0]), evidence_hash(&b[0]));
        f.commits_after_publish[0].sha = "d".repeat(40);
        let c = infer_events(&f);
        assert_ne!(evidence_hash(&a[0]), evidence_hash(&c[0]));
    }

    /// Scripted fixture: 30 findings, five per outcome, derived outcomes match 100%.
    #[test]
    fn scripted_fixture_matches_labels() {
        let mut cases: Vec<(FindingFacts, Outcome)> = Vec::new();
        for i in 0..5 {
            let mut accepted = facts();
            accepted.commits_after_publish = vec![touch(&["a.run"], &[])];
            cases.push((accepted, Outcome::Accepted));
            let mut rejected = facts();
            rejected.feedback = vec![(Verdict::NotRelevant, t(i))];
            cases.push((rejected, Outcome::Rejected));
            let mut dismissed = facts();
            dismissed.feedback = vec![(Verdict::Intentional, t(i))];
            cases.push((dismissed, Outcome::Dismissed));
            let mut resolved = facts();
            resolved.thread_resolved = true;
            cases.push((resolved, Outcome::ResolvedUnattributed));
            let mut ignored = facts();
            ignored.pr_closed_or_merged = true;
            cases.push((ignored, Outcome::Ignored));
            cases.push((facts(), Outcome::Pending));
        }
        assert_eq!(cases.len(), 30);
        for (f, expected) in &cases {
            assert_eq!(derive_outcome(f), *expected);
        }
    }
}
