//! `PublishedFinding`: a verified finding as delivered to the provider.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::ids::{CommitSha, OrganizationId, PublishedFindingId, VerifiedFindingId};
use crate::location::SourceLocation;

/// Where a finding appears on the provider. `Summary` carries an out-of-diff finding: it moves to
/// the summary comment and is never dropped (legacy `policy.rs:128-137`, INV-015). The legacy
/// `Discarded` placement is now a suppression state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Inline,
    Summary,
}

/// Maps onto `published_findings` (DOM-009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishedFinding {
    pub id: PublishedFindingId,
    pub verified_finding_id: VerifiedFindingId,
    pub organization_id: OrganizationId,
    pub placement: Placement,
    /// Present exactly when the placement is `Inline`.
    pub location: Option<SourceLocation>,
    pub head_sha: CommitSha,
    pub provider_review_id: Option<String>,
    pub provider_comment_id: Option<String>,
    pub published_at: DateTime<Utc>,
}

impl PublishedFinding {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: PublishedFindingId,
        verified_finding_id: VerifiedFindingId,
        organization_id: OrganizationId,
        placement: Placement,
        location: Option<SourceLocation>,
        head_sha: CommitSha,
        provider_review_id: Option<String>,
        provider_comment_id: Option<String>,
        published_at: DateTime<Utc>,
    ) -> Result<Self, CoreError> {
        Self {
            id,
            verified_finding_id,
            organization_id,
            placement,
            location,
            head_sha,
            provider_review_id,
            provider_comment_id,
            published_at,
        }
        .validated()
    }

    /// `location` must be present exactly when the placement is `Inline`.
    pub fn validated(self) -> Result<Self, CoreError> {
        let inline = self.placement == Placement::Inline;
        if inline != self.location.is_some() {
            return Err(CoreError::InvalidId {
                kind: "PublishedFinding",
                reason: if inline {
                    "an inline placement requires a location".to_owned()
                } else {
                    "a summary placement must not carry a location".to_owned()
                },
            });
        }
        Ok(self)
    }
}

impl<'de> Deserialize<'de> for PublishedFinding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            id: PublishedFindingId,
            verified_finding_id: VerifiedFindingId,
            organization_id: OrganizationId,
            placement: Placement,
            location: Option<SourceLocation>,
            head_sha: CommitSha,
            provider_review_id: Option<String>,
            provider_comment_id: Option<String>,
            published_at: DateTime<Utc>,
        }
        let r = Raw::deserialize(deserializer)?;
        Self {
            id: r.id,
            verified_finding_id: r.verified_finding_id,
            organization_id: r.organization_id,
            placement: r.placement,
            location: r.location,
            head_sha: r.head_sha,
            provider_review_id: r.provider_review_id,
            provider_comment_id: r.provider_comment_id,
            published_at: r.published_at,
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

    fn location() -> SourceLocation {
        SourceLocation {
            path: RepoPath::new("src/a.ts").unwrap(),
            side: DiffSide::Head,
            lines: LineRange::new(3, 3).unwrap(),
            range: None,
        }
    }

    fn make(
        placement: Placement,
        location: Option<SourceLocation>,
    ) -> Result<PublishedFinding, CoreError> {
        PublishedFinding::new(
            PublishedFindingId::new(),
            VerifiedFindingId::new(),
            OrganizationId::new(),
            placement,
            location,
            "a".repeat(40).parse().unwrap(),
            Some("review-1".into()),
            None,
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        )
    }

    #[test]
    fn inline_placement_requires_location() {
        assert!(make(Placement::Inline, None).is_err());
        assert!(make(Placement::Inline, Some(location())).is_ok());
        assert!(make(Placement::Summary, None).is_ok());
        assert!(make(Placement::Summary, Some(location())).is_err());
    }

    #[test]
    fn published_finding_deserialize_validates() {
        let ok = make(Placement::Summary, None).unwrap();
        let json = serde_json::to_string(&ok).unwrap();
        assert_eq!(serde_json::from_str::<PublishedFinding>(&json).unwrap(), ok);
        let bad = json.replace("\"summary\"", "\"inline\"");
        assert!(serde_json::from_str::<PublishedFinding>(&bad).is_err());
    }
}
