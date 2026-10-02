//! `VerifiedFinding`: a candidate that went through verification (ADR-011).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::severity::{Confidence, Severity};
use crate::evidence::Evidence;
use crate::ids::{CandidateFindingId, OrganizationId, VerifiedFindingId};
use crate::version::VerificationVersion;

/// The PRD §55 publication bands. The thresholds are applied in VER-010.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PublicationBand {
    // Below 0.55.
    Suppress,
    // 0.55 to 0.70: stored and shown in the UI only.
    Internal,
    // 0.70 to 0.85: published only at medium severity or above.
    PublishIfMediumOrAbove,
    // Above 0.85.
    Publish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StageOutcome {
    Pass,
    Fail,
    Inconclusive,
}

/// What one of the eight verification stages decided.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageOutcomeRecord {
    pub stage: u8,
    pub outcome: StageOutcome,
    pub reason: Option<String>,
}

/// A candidate with a *computed* confidence (never the model's self-report), a final severity
/// and a publication band. Maps onto `verified_findings` (DOM-009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifiedFinding {
    pub id: VerifiedFindingId,
    pub candidate_id: CandidateFindingId,
    pub organization_id: OrganizationId,
    pub computed_confidence: Confidence,
    pub severity: Severity,
    pub band: PublicationBand,
    pub verification_version: VerificationVersion,
    pub stage_outcomes: Vec<StageOutcomeRecord>,
    pub evidence: Vec<Evidence>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_and_outcome_wire_names() {
        let bands = [
            (PublicationBand::Suppress, "suppress"),
            (PublicationBand::Internal, "internal"),
            (
                PublicationBand::PublishIfMediumOrAbove,
                "publish_if_medium_or_above",
            ),
            (PublicationBand::Publish, "publish"),
        ];
        for (band, name) in bands {
            assert_eq!(serde_json::to_string(&band).unwrap(), format!("\"{name}\""));
        }
        assert_eq!(
            serde_json::to_string(&StageOutcome::Inconclusive).unwrap(),
            "\"inconclusive\""
        );
    }

    #[test]
    fn verified_finding_roundtrips_and_validates_confidence() {
        let v = VerifiedFinding {
            id: VerifiedFindingId::new(),
            candidate_id: CandidateFindingId::new(),
            organization_id: OrganizationId::new(),
            computed_confidence: Confidence::new(0.8).unwrap(),
            severity: Severity::High,
            band: PublicationBand::PublishIfMediumOrAbove,
            verification_version: VerificationVersion(1),
            stage_outcomes: vec![StageOutcomeRecord {
                stage: 3,
                outcome: StageOutcome::Pass,
                reason: None,
            }],
            evidence: vec![],
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<VerifiedFinding>(&json).unwrap(), v);
        let bad = json.replace("\"computed_confidence\":0.8", "\"computed_confidence\":1.8");
        assert_ne!(bad, json);
        assert!(serde_json::from_str::<VerifiedFinding>(&bad).is_err());
    }
}
