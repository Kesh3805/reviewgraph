//! Pull-request shell entity (DOM-005).
//!
//! Populated by the provider integrations (API-006) and refreshed by webhooks. Maps onto
//! `pull_requests` (DOM-009).
//!
//! `title` and `author_login` are provider data. They are stored, but must never be
//! interpolated into model prompts without the redaction and escaping done by REV-001.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{CommitSha, OrganizationId, PullRequestId, RepositoryId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PullRequest {
    pub id: PullRequestId,
    pub organization_id: OrganizationId,
    pub repository_id: RepositoryId,
    pub provider_number: u64,
    pub title: String,
    pub author_login: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_sha: CommitSha,
    pub head_sha: CommitSha,
    pub merge_base_sha: Option<CommitSha>,
    pub state: PrState,
    pub draft: bool,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_state_wire_names() {
        for (state, name) in [
            (PrState::Open, "open"),
            (PrState::Closed, "closed"),
            (PrState::Merged, "merged"),
        ] {
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(
                serde_json::from_str::<PrState>(&format!("\"{name}\"")).unwrap(),
                state
            );
        }
        assert!(serde_json::from_str::<PrState>("\"OPEN\"").is_err());
    }
}
