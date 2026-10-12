//! Rule violations as deterministic evidence (POL-005).
//!
//! Each violation becomes a candidate finding of its own (reviewer `deterministic:policy`,
//! recorded in the candidate's reasoning artifact) and is attached as evidence to every model
//! candidate that makes the same claim: same anchor path within ±3 lines or the same symbol, and
//! the same topic (category). Deduplication then merges them. A deterministic candidate is still
//! a candidate: it passes through every verification stage (INV-002).

use review_core::error::CoreError;
use review_core::finding::{
    ArtifactKind, CandidateDraft, CandidateFinding, FindingArtifact, FindingCategory, ReviewerType,
};
use review_core::ids::{OrganizationId, ReviewRunId, ReviewerRunId, SymbolId};
use review_core::location::ContentHash;

use super::evaluator::RuleViolation;
use super::evidence::location;

/// The reviewer label deterministic policy candidates carry in the candidate pool.
pub const POLICY_REVIEWER: &str = "deterministic:policy";
/// How far apart an anchor may be and still overlap.
pub const OVERLAP_LINES: u32 = 3;

/// The ids a candidate needs.
#[derive(Debug, Clone, Copy)]
pub struct CandidateIds {
    pub organization_id: OrganizationId,
    pub review_run_id: ReviewRunId,
    pub reviewer_run_id: ReviewerRunId,
}

/// The finding category of a rule key (its topic).
pub fn category_of(rule_key: &str) -> FindingCategory {
    match rule_key {
        "forbidden_dependency" => FindingCategory::Architecture,
        "migrations_only" => FindingCategory::DataIntegrity,
        "public_api_changes_require_tests" => FindingCategory::Testing,
        "queue_deterministic_job_id" => FindingCategory::Concurrency,
        _ => FindingCategory::Correctness,
    }
}

/// The reviewer type that owns the topic (the closed `ReviewerType` set has no deterministic
/// member; the `deterministic:policy` label travels in the reasoning artifact).
pub fn reviewer_of(rule_key: &str) -> ReviewerType {
    category_of(rule_key).primary_reviewer()
}

fn title(v: &RuleViolation) -> String {
    let base = match v.rule_key.as_str() {
        "forbidden_dependency" => format!("Forbidden dependency ({})", v.rule_id),
        "migrations_only" => "Schema change outside a migration".to_owned(),
        "public_api_changes_require_tests" => "API contract changed without tests".to_owned(),
        "queue_deterministic_job_id" => "Job enqueued without a deterministic jobId".to_owned(),
        _ => format!("Rule {} violated", v.rule_id),
    };
    base.chars().take(200).collect()
}

/// Turns a violation into a candidate draft.
pub fn violation_to_draft(v: &RuleViolation, ids: CandidateIds) -> Result<CandidateDraft, CoreError> {
    let changed_location = location(&v.anchor.path, v.anchor.start_line, v.anchor.end_line)
        .ok_or_else(|| CoreError::InvalidId {
            kind: "RuleViolation",
            reason: format!("anchor {}:{} is not a valid location", v.anchor.path, v.anchor.start_line),
        })?;
    let summary = format!(
        "{POLICY_REVIEWER} rule_id={} rule_key={}",
        v.rule_id, v.rule_key
    );
    let artifact = FindingArtifact::new(
        ArtifactKind::ToolOutput,
        summary.clone(),
        ContentHash::of(summary.as_bytes()),
        None,
    )?;
    Ok(CandidateDraft {
        organization_id: ids.organization_id,
        review_run_id: ids.review_run_id,
        reviewer_run_id: ids.reviewer_run_id,
        category: category_of(&v.rule_key),
        title: title(v),
        description: v.message.chars().take(8000).collect(),
        changed_location,
        evidence: v.evidence.clone(),
        affected_symbols: v
            .symbol_key
            .iter()
            .map(SymbolId::from_canonical_unchecked)
            .collect(),
        severity_candidate: v.severity.to_core(),
        confidence_candidate: None,
        reviewer: reviewer_of(&v.rule_key),
        reasoning_artifacts: vec![artifact],
    })
}

/// Converts every violation; violations whose anchor is invalid are skipped (and counted).
pub fn violations_to_drafts(violations: &[RuleViolation], ids: CandidateIds) -> Vec<CandidateDraft> {
    let drafts: Vec<CandidateDraft> = violations
        .iter()
        .filter_map(|v| violation_to_draft(v, ids).ok())
        .collect();
    crate::metrics::candidate_findings(POLICY_REVIEWER, drafts.len());
    drafts
}

/// Whether a violation and a candidate make the same claim.
pub fn overlaps(v: &RuleViolation, candidate: &CandidateFinding) -> bool {
    if candidate.category != category_of(&v.rule_key) {
        return false;
    }
    let loc = &candidate.changed_location;
    let same_anchor = loc.path.as_str() == v.anchor.path
        && loc.lines.start <= v.anchor.end_line + OVERLAP_LINES
        && v.anchor.start_line <= loc.lines.end + OVERLAP_LINES;
    let same_symbol = v
        .symbol_key
        .as_deref()
        .is_some_and(|key| candidate.affected_symbols.iter().any(|s| s.as_str() == key));
    same_anchor || same_symbol
}

/// Attaches each violation's evidence to every overlapping model candidate. Returns how many
/// candidates received evidence.
pub fn attach_to_overlapping(violations: &[RuleViolation], candidates: &mut [CandidateFinding]) -> usize {
    let mut touched = 0;
    for candidate in candidates.iter_mut() {
        let mut attached = false;
        for v in violations.iter().filter(|v| overlaps(v, candidate)) {
            for evidence in &v.evidence {
                if !candidate.evidence.contains(evidence) {
                    candidate.evidence.push(evidence.clone());
                    attached = true;
                }
            }
        }
        touched += usize::from(attached);
    }
    touched
}
