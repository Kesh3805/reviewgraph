//! `CandidateFinding`: what a reviewer proposes, before verification (PRD §49).

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::severity::{Confidence, FindingCategory, Severity};
use super::state::FindingState;
use crate::error::CoreError;
use crate::evidence::Evidence;
use crate::ids::{CandidateFindingId, OrganizationId, ReviewRunId, ReviewerRunId, SymbolId};
use crate::location::{ContentHash, SourceLocation};
use crate::reviewer_type::ReviewerType;

const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 8000;
const MAX_ARTIFACT_SUMMARY_CHARS: usize = 2000;
const MAX_BLOB_KEY_CHARS: usize = 512;
const MAX_SUPPRESSION_DETAIL_CHARS: usize = 1000;

fn check_chars(field: &'static str, s: &str, min: usize, max: usize) -> Result<(), CoreError> {
    let n = s.chars().count();
    if n < min || n > max {
        return Err(CoreError::OutOfRange {
            field,
            value: format!("{n} characters (allowed {min}..={max})"),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    ModelRationale,
    ToolOutput,
    GraphQuery,
}

/// A reasoning artifact attached to a candidate. There is deliberately no field that can hold a
/// prompt: prompts are never persisted in clear text. Large content lives in the object store
/// under `blob_key`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingArtifact {
    pub kind: ArtifactKind,
    /// At most 2000 characters.
    pub summary: String,
    pub content_hash: ContentHash,
    pub blob_key: Option<String>,
}

impl FindingArtifact {
    pub fn new(
        kind: ArtifactKind,
        summary: impl Into<String>,
        content_hash: ContentHash,
        blob_key: Option<String>,
    ) -> Result<Self, CoreError> {
        Self {
            kind,
            summary: summary.into(),
            content_hash,
            blob_key,
        }
        .validated()
    }

    fn validated(self) -> Result<Self, CoreError> {
        check_chars(
            "finding_artifact.summary",
            &self.summary,
            0,
            MAX_ARTIFACT_SUMMARY_CHARS,
        )?;
        if let Some(key) = &self.blob_key {
            check_chars("finding_artifact.blob_key", key, 1, MAX_BLOB_KEY_CHARS)?;
        }
        Ok(self)
    }
}

impl<'de> Deserialize<'de> for FindingArtifact {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            kind: ArtifactKind,
            summary: String,
            content_hash: ContentHash,
            blob_key: Option<String>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            kind: r.kind,
            summary: r.summary,
            content_hash: r.content_hash,
            blob_key: r.blob_key,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

/// Stable identity of a finding within one reviewer run: `v1:` plus 32 hex characters.
///
/// `blake3("v1\0{reviewer}\0{category}\0{path}\0{line_start}\0{normalized_title}")[..16]`, where
/// normalization lowercases, collapses whitespace and trims. **Severity is deliberately
/// excluded**: the legacy store fingerprinted the severity label, so a re-rated finding counted
/// as new. DED-001's root-cause fingerprint replaces v1 for cross-reviewer deduplication.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FindingFingerprint(String);

impl FindingFingerprint {
    pub fn v1(
        reviewer: ReviewerType,
        category: FindingCategory,
        path: &str,
        line_start: u32,
        title: &str,
    ) -> Self {
        let normalized = title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let input = format!(
            "v1\0{}\0{}\0{}\0{}\0{}",
            reviewer.as_str(),
            category.as_str(),
            path,
            line_start,
            normalized
        );
        let hash = blake3::hash(input.as_bytes());
        Self(format!("v1:{}", hex::encode(&hash.as_bytes()[..16])))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FindingFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for FindingFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FindingFingerprint({})", self.0)
    }
}

impl FromStr for FindingFingerprint {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ok = s.strip_prefix("v1:").is_some_and(|h| {
            h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        });
        if !ok {
            return Err(CoreError::InvalidId {
                kind: "FindingFingerprint",
                reason: "expected v1: followed by 32 lowercase hex characters".to_owned(),
            });
        }
        Ok(Self(s.to_owned()))
    }
}

impl Serialize for FindingFingerprint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for FindingFingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for FindingFingerprint {
    fn schema_name() -> String {
        "FindingFingerprint".to_owned()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        crate::schema::string_pattern("FindingFingerprint", "^v1:[0-9a-f]{32}$")
    }
}

/// Why a finding left the happy path. The variant must match the target state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SuppressionReason {
    LowConfidence {
        computed: Confidence,
        threshold: Confidence,
    },
    Duplicate {
        of: CandidateFindingId,
    },
    Preexisting,
    NotActionable {
        gate: String,
    },
    Policy {
        rule: String,
    },
    Invalidated {
        superseded_by: Option<ReviewRunId>,
    },
}

impl SuppressionReason {
    /// The only state this reason can accompany.
    pub const fn target_state(&self) -> FindingState {
        match self {
            Self::LowConfidence { .. } => FindingState::SuppressedLowConfidence,
            Self::Duplicate { .. } => FindingState::SuppressedDuplicate,
            Self::Preexisting => FindingState::SuppressedPreexisting,
            Self::NotActionable { .. } => FindingState::SuppressedNotActionable,
            Self::Policy { .. } => FindingState::SuppressedPolicy,
            Self::Invalidated { .. } => FindingState::Invalidated,
        }
    }
}

/// A persisted suppression: reason, detail (at most 1000 characters; no source, prompts or
/// tokens) and the verification stage (1..=8) that decided it, if any.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Suppression {
    pub reason: SuppressionReason,
    pub detail: String,
    pub stage: Option<u8>,
}

impl Suppression {
    pub fn new(
        reason: SuppressionReason,
        detail: impl Into<String>,
        stage: Option<u8>,
    ) -> Result<Self, CoreError> {
        Self {
            reason,
            detail: detail.into(),
            stage,
        }
        .validated()
    }

    fn validated(self) -> Result<Self, CoreError> {
        check_chars(
            "suppression.detail",
            &self.detail,
            0,
            MAX_SUPPRESSION_DETAIL_CHARS,
        )?;
        if let Some(stage) = self.stage {
            if !(1..=8).contains(&stage) {
                return Err(CoreError::OutOfRange {
                    field: "suppression.stage",
                    value: stage.to_string(),
                });
            }
        }
        Ok(self)
    }
}

impl<'de> Deserialize<'de> for Suppression {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            reason: SuppressionReason,
            detail: String,
            stage: Option<u8>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            reason: r.reason,
            detail: r.detail,
            stage: r.stage,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

/// The reviewer-supplied part of a candidate; [`CandidateFinding::new`] adds identity, the
/// fingerprint and the initial `GENERATED` state.
#[derive(Debug, Clone)]
pub struct CandidateDraft {
    pub organization_id: OrganizationId,
    pub review_run_id: ReviewRunId,
    pub reviewer_run_id: ReviewerRunId,
    pub category: FindingCategory,
    pub title: String,
    pub description: String,
    pub changed_location: SourceLocation,
    pub evidence: Vec<Evidence>,
    pub affected_symbols: Vec<SymbolId>,
    pub severity_candidate: Severity,
    pub confidence_candidate: Option<Confidence>,
    pub reviewer: ReviewerType,
    pub reasoning_artifacts: Vec<FindingArtifact>,
}

/// A finding proposed by a reviewer (PRD §49). Maps onto `candidate_findings` (DOM-009).
///
/// `confidence_candidate` is the model's self-report: **informational only**, never a
/// publication input (PRD §54, ADR-011). Text fields are length-capped to bound storage abuse
/// from model output.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateFinding {
    pub id: CandidateFindingId,
    pub organization_id: OrganizationId,
    pub review_run_id: ReviewRunId,
    pub reviewer_run_id: ReviewerRunId,
    pub category: FindingCategory,
    /// 1 to 200 characters.
    pub title: String,
    /// At most 8000 characters.
    pub description: String,
    pub changed_location: SourceLocation,
    pub evidence: Vec<Evidence>,
    pub affected_symbols: Vec<SymbolId>,
    pub severity_candidate: Severity,
    pub confidence_candidate: Option<Confidence>,
    pub reviewer: ReviewerType,
    pub reasoning_artifacts: Vec<FindingArtifact>,
    pub fingerprint: FindingFingerprint,
    pub state: FindingState,
    pub suppression: Option<Suppression>,
    pub created_at: DateTime<Utc>,
}

impl CandidateFinding {
    /// Creates a candidate in `GENERATED` with its v1 fingerprint. The ID is generated once, here,
    /// and never regenerated on retry.
    pub fn new(
        id: CandidateFindingId,
        draft: CandidateDraft,
        created_at: DateTime<Utc>,
    ) -> Result<Self, CoreError> {
        let fingerprint = FindingFingerprint::v1(
            draft.reviewer,
            draft.category,
            draft.changed_location.path.as_str(),
            draft.changed_location.lines.start,
            &draft.title,
        );
        Self {
            id,
            organization_id: draft.organization_id,
            review_run_id: draft.review_run_id,
            reviewer_run_id: draft.reviewer_run_id,
            category: draft.category,
            title: draft.title,
            description: draft.description,
            changed_location: draft.changed_location,
            evidence: draft.evidence,
            affected_symbols: draft.affected_symbols,
            severity_candidate: draft.severity_candidate,
            confidence_candidate: draft.confidence_candidate,
            reviewer: draft.reviewer,
            reasoning_artifacts: draft.reasoning_artifacts,
            fingerprint,
            state: FindingState::Generated,
            suppression: None,
            created_at,
        }
        .validated()
    }

    /// Checks text caps, that the fingerprint matches the content, and that `suppression` is
    /// present exactly when the state is suppressed or `INVALIDATED`, with a matching reason.
    pub fn validated(self) -> Result<Self, CoreError> {
        check_chars("candidate.title", &self.title, 1, MAX_TITLE_CHARS)?;
        check_chars(
            "candidate.description",
            &self.description,
            0,
            MAX_DESCRIPTION_CHARS,
        )?;
        let expected = FindingFingerprint::v1(
            self.reviewer,
            self.category,
            self.changed_location.path.as_str(),
            self.changed_location.lines.start,
            &self.title,
        );
        if expected != self.fingerprint {
            return Err(CoreError::InvalidId {
                kind: "FindingFingerprint",
                reason: "fingerprint does not match the finding content".to_owned(),
            });
        }
        let needs_suppression =
            self.state.is_suppressed() || self.state == FindingState::Invalidated;
        let consistent = match &self.suppression {
            Some(s) => needs_suppression && s.reason.target_state() == self.state,
            None => !needs_suppression,
        };
        if !consistent {
            return Err(CoreError::InvalidTransition {
                entity: "CandidateFinding",
                from: self.state.as_str(),
                to: self.state.as_str(),
            });
        }
        Ok(self)
    }

    /// Moves along an edge of the lifecycle table. `suppression` must be `Some` exactly when `to`
    /// is a suppressed state or `INVALIDATED`, and its reason must match `to`. Transitioning to the
    /// current state is an error here; the SQL compare-and-set turns duplicates into no-ops.
    pub fn transition(
        &mut self,
        to: FindingState,
        suppression: Option<Suppression>,
    ) -> Result<(), CoreError> {
        let invalid = || CoreError::InvalidTransition {
            entity: "CandidateFinding",
            from: self.state.as_str(),
            to: to.as_str(),
        };
        if !self.state.can_transition_to(to) {
            return Err(invalid());
        }
        let needs_suppression = to.is_suppressed() || to == FindingState::Invalidated;
        match (&suppression, needs_suppression) {
            (Some(s), true) if s.reason.target_state() == to => {}
            (None, false) => {}
            _ => return Err(invalid()),
        }
        self.state = to;
        self.suppression = suppression;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CandidateFinding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            id: CandidateFindingId,
            organization_id: OrganizationId,
            review_run_id: ReviewRunId,
            reviewer_run_id: ReviewerRunId,
            category: FindingCategory,
            title: String,
            description: String,
            changed_location: SourceLocation,
            evidence: Vec<Evidence>,
            affected_symbols: Vec<SymbolId>,
            severity_candidate: Severity,
            confidence_candidate: Option<Confidence>,
            reviewer: ReviewerType,
            reasoning_artifacts: Vec<FindingArtifact>,
            fingerprint: FindingFingerprint,
            state: FindingState,
            suppression: Option<Suppression>,
            created_at: DateTime<Utc>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            id: r.id,
            organization_id: r.organization_id,
            review_run_id: r.review_run_id,
            reviewer_run_id: r.reviewer_run_id,
            category: r.category,
            title: r.title,
            description: r.description,
            changed_location: r.changed_location,
            evidence: r.evidence,
            affected_symbols: r.affected_symbols,
            severity_candidate: r.severity_candidate,
            confidence_candidate: r.confidence_candidate,
            reviewer: r.reviewer,
            reasoning_artifacts: r.reasoning_artifacts,
            fingerprint: r.fingerprint,
            state: r.state,
            suppression: r.suppression,
            created_at: r.created_at,
        }
        .validated()
        .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::{DiffSide, LineRange, RepoPath};
    use chrono::TimeZone;

    fn location(line: u32) -> SourceLocation {
        SourceLocation {
            path: RepoPath::new("src/auth/auth.service.ts").unwrap(),
            side: DiffSide::Head,
            lines: LineRange::new(line, line + 2).unwrap(),
            range: None,
        }
    }

    fn draft(title: &str) -> CandidateDraft {
        CandidateDraft {
            organization_id: OrganizationId::new(),
            review_run_id: ReviewRunId::new(),
            reviewer_run_id: ReviewerRunId::new(),
            category: FindingCategory::Security,
            title: title.to_owned(),
            description: "The guard is skipped when the header is absent.".to_owned(),
            changed_location: location(10),
            evidence: vec![],
            affected_symbols: vec![],
            severity_candidate: Severity::High,
            confidence_candidate: Some(Confidence::new(0.9).unwrap()),
            reviewer: ReviewerType::Security,
            reasoning_artifacts: vec![],
        }
    }

    fn candidate() -> CandidateFinding {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        CandidateFinding::new(CandidateFindingId::new(), draft("Authorization bypass"), at).unwrap()
    }

    #[test]
    fn fingerprint_v1_golden() {
        let fp = FindingFingerprint::v1(
            ReviewerType::Security,
            FindingCategory::Security,
            "src/auth/auth.service.ts",
            10,
            "Authorization bypass",
        );
        assert_eq!(fp.as_str(), "v1:0c12182303a0432ce513dadc368ab1ca");
        assert_eq!(fp.as_str().parse::<FindingFingerprint>().unwrap(), fp);
        assert!("v2:00112233445566778899aabbccddeeff"
            .parse::<FindingFingerprint>()
            .is_err());
        assert!("v1:00112233445566778899AABBCCDDEEFF"
            .parse::<FindingFingerprint>()
            .is_err());
        assert!("v1:0011".parse::<FindingFingerprint>().is_err());
    }

    #[test]
    fn fingerprint_ignores_severity() {
        let a = candidate();
        let mut b_draft = draft("Authorization bypass");
        b_draft.severity_candidate = Severity::Low;
        let b = CandidateFinding::new(CandidateFindingId::new(), b_draft, a.created_at).unwrap();
        assert_ne!(a.severity_candidate, b.severity_candidate);
        assert_eq!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn fingerprint_normalizes_title_whitespace_and_case() {
        let fp = |t: &str| {
            FindingFingerprint::v1(ReviewerType::Test, FindingCategory::Testing, "a.ts", 3, t)
        };
        assert_eq!(fp("Missing  Null\tCheck "), fp("missing null check"));
        assert_eq!(fp("  MISSING NULL CHECK\n"), fp("missing null check"));
        assert_ne!(fp("missing null check"), fp("missing null checks"));
        // Every input component participates.
        let base =
            FindingFingerprint::v1(ReviewerType::Test, FindingCategory::Testing, "a.ts", 3, "t");
        assert_ne!(
            base,
            FindingFingerprint::v1(
                ReviewerType::Security,
                FindingCategory::Testing,
                "a.ts",
                3,
                "t"
            )
        );
        assert_ne!(
            base,
            FindingFingerprint::v1(
                ReviewerType::Test,
                FindingCategory::Security,
                "a.ts",
                3,
                "t"
            )
        );
        assert_ne!(
            base,
            FindingFingerprint::v1(ReviewerType::Test, FindingCategory::Testing, "b.ts", 3, "t")
        );
        assert_ne!(
            base,
            FindingFingerprint::v1(ReviewerType::Test, FindingCategory::Testing, "a.ts", 4, "t")
        );
    }

    fn suppression(reason: SuppressionReason) -> Option<Suppression> {
        Some(Suppression::new(reason, "detail", Some(5)).unwrap())
    }

    #[test]
    fn suppression_required_iff_suppressed_target() {
        // Suppressed target without a suppression.
        let mut c = candidate();
        assert!(c.transition(FindingState::SuppressedPolicy, None).is_err());
        assert_eq!(c.state, FindingState::Generated);
        // Non-suppressed target with a suppression.
        let mut c = candidate();
        let s = suppression(SuppressionReason::Policy { rule: "x".into() });
        assert!(c.transition(FindingState::EvidenceCollected, s).is_err());
        assert_eq!(c.state, FindingState::Generated);
        // Invalidated also needs one.
        let mut c = candidate();
        assert!(c.transition(FindingState::Invalidated, None).is_err());
        let inv = suppression(SuppressionReason::Invalidated {
            superseded_by: None,
        });
        c.transition(FindingState::Invalidated, inv).unwrap();
        assert_eq!(c.state, FindingState::Invalidated);
        assert!(c.suppression.is_some());
        // Happy path leaves suppression empty.
        let mut c = candidate();
        c.transition(FindingState::EvidenceCollected, None).unwrap();
        assert!(c.suppression.is_none());
    }

    #[test]
    fn suppression_reason_must_match_state() {
        let reasons = [
            SuppressionReason::LowConfidence {
                computed: Confidence::new(0.3).unwrap(),
                threshold: Confidence::new(0.55).unwrap(),
            },
            SuppressionReason::Duplicate {
                of: CandidateFindingId::new(),
            },
            SuppressionReason::Preexisting,
            SuppressionReason::NotActionable {
                gate: "stage7".into(),
            },
            SuppressionReason::Policy { rule: "cap".into() },
            SuppressionReason::Invalidated {
                superseded_by: Some(ReviewRunId::new()),
            },
        ];
        let targets = [
            FindingState::SuppressedLowConfidence,
            FindingState::SuppressedDuplicate,
            FindingState::SuppressedPreexisting,
            FindingState::SuppressedNotActionable,
            FindingState::SuppressedPolicy,
            FindingState::Invalidated,
        ];
        for (ri, reason) in reasons.iter().enumerate() {
            assert_eq!(reason.target_state(), targets[ri]);
            for (ti, target) in targets.iter().enumerate() {
                // Walk to a state from which `target` is legal.
                let mut c = candidate();
                c.transition(FindingState::EvidenceCollected, None).unwrap();
                if matches!(target, FindingState::SuppressedDuplicate) {
                    c.transition(FindingState::Verified, None).unwrap();
                }
                let result = c.transition(*target, suppression(reason.clone()));
                assert_eq!(result.is_ok(), ri == ti, "{reason:?} -> {target}");
            }
        }
    }

    #[test]
    fn transition_follows_table_and_reports_conflict() {
        let mut c = candidate();
        let err = c.transition(FindingState::Published, None).unwrap_err();
        assert!(matches!(
            err,
            CoreError::InvalidTransition {
                entity: "CandidateFinding",
                from: "GENERATED",
                to: "PUBLISHED"
            }
        ));
        // Same-state transitions are errors at this layer.
        assert!(c.transition(FindingState::Generated, None).is_err());
        for next in [
            FindingState::EvidenceCollected,
            FindingState::Verified,
            FindingState::Deduplicated,
            FindingState::Prioritized,
            FindingState::Published,
        ] {
            c.transition(next, None).unwrap();
        }
        assert!(c
            .transition(
                FindingState::Invalidated,
                suppression(SuppressionReason::Invalidated {
                    superseded_by: None
                })
            )
            .is_err());
    }

    #[test]
    fn suppression_validates_stage_and_detail() {
        assert!(Suppression::new(SuppressionReason::Preexisting, "x", Some(0)).is_err());
        assert!(Suppression::new(SuppressionReason::Preexisting, "x", Some(9)).is_err());
        assert!(Suppression::new(SuppressionReason::Preexisting, "x", Some(8)).is_ok());
        assert!(Suppression::new(SuppressionReason::Preexisting, "x", None).is_ok());
        assert!(Suppression::new(SuppressionReason::Preexisting, "x".repeat(1001), None).is_err());
    }

    #[test]
    fn text_fields_are_capped() {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        assert!(CandidateFinding::new(CandidateFindingId::new(), draft(""), at).is_err());
        assert!(
            CandidateFinding::new(CandidateFindingId::new(), draft(&"t".repeat(200)), at).is_ok()
        );
        assert!(
            CandidateFinding::new(CandidateFindingId::new(), draft(&"t".repeat(201)), at).is_err()
        );
        let mut d = draft("ok");
        d.description = "d".repeat(8001);
        assert!(CandidateFinding::new(CandidateFindingId::new(), d, at).is_err());
        let hash = ContentHash::of(b"x");
        assert!(
            FindingArtifact::new(ArtifactKind::ToolOutput, "s".repeat(2001), hash, None).is_err()
        );
        assert!(
            FindingArtifact::new(ArtifactKind::ToolOutput, "s".repeat(2000), hash, None).is_ok()
        );
    }

    #[test]
    fn candidate_roundtrips_and_deserialize_validates() {
        let mut c = candidate();
        c.transition(FindingState::EvidenceCollected, None).unwrap();
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<CandidateFinding>(&json).unwrap(), c);

        // A tampered title no longer matches the fingerprint.
        let tampered = json.replace("Authorization bypass", "Something else");
        assert!(serde_json::from_str::<CandidateFinding>(&tampered).is_err());
        // A suppressed state without a suppression is rejected.
        let bad_state = json.replace("\"EVIDENCE_COLLECTED\"", "\"SUPPRESSED_POLICY\"");
        assert!(serde_json::from_str::<CandidateFinding>(&bad_state).is_err());
    }
}
