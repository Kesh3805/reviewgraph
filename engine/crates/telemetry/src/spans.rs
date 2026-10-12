//! The lifecycle span registry (OBS-004): the 15 canonical span names of target-architecture §8,
//! their parents, owners and required attributes. `docs/operations/span-catalog.md` is rendered
//! from this table.
//!
//! Lifecycle spans are created with [`stage_span!`], which only accepts a [`SpanName`], so a
//! stage cannot drift to a free-form name.

use crate::attrs::{ORGANIZATION_ID, REPOSITORY_ID, REVIEW_RUN_ID};

/// A lifecycle stage span name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SpanName {
    WebhookReceived,
    RepositoryCheckout,
    RepositoryIndex,
    IncrementalGraphUpdate,
    DiffAnalysis,
    SymbolMapping,
    ImpactAnalysis,
    ContextSelection,
    QdrantSearch,
    ReviewerExecution,
    ModelRequest,
    CandidateGenerated,
    FindingVerification,
    Deduplication,
    Publication,
}

/// Where a span sits in the review trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parent {
    /// The trace root (the webhook handler).
    Root,
    /// The `job` span of a queue consumer.
    Job(&'static str),
    /// Another lifecycle span.
    Span(SpanName),
}

/// Attributes every lifecycle span carries.
pub const BASE_ATTRS: [&str; 3] = [ORGANIZATION_ID, REPOSITORY_ID, REVIEW_RUN_ID];

/// Attribute recorded on every lifecycle span when it ends: `ok|skipped|failed|superseded`.
pub const OUTCOME_ATTR: &str = "rg.outcome";

/// Attribute keys no span may carry (source, diff and prompt content never go on spans).
pub const FORBIDDEN_ATTRS: [&str; 4] = ["prompt", "source", "diff", "body"];

impl SpanName {
    pub const ALL: [SpanName; 15] = [
        Self::WebhookReceived,
        Self::RepositoryCheckout,
        Self::RepositoryIndex,
        Self::IncrementalGraphUpdate,
        Self::DiffAnalysis,
        Self::SymbolMapping,
        Self::ImpactAnalysis,
        Self::ContextSelection,
        Self::QdrantSearch,
        Self::ReviewerExecution,
        Self::ModelRequest,
        Self::CandidateGenerated,
        Self::FindingVerification,
        Self::Deduplication,
        Self::Publication,
    ];

    /// The wire name used for the span.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WebhookReceived => "webhook_received",
            Self::RepositoryCheckout => "repository_checkout",
            Self::RepositoryIndex => "repository_index",
            Self::IncrementalGraphUpdate => "incremental_graph_update",
            Self::DiffAnalysis => "diff_analysis",
            Self::SymbolMapping => "symbol_mapping",
            Self::ImpactAnalysis => "impact_analysis",
            Self::ContextSelection => "context_selection",
            Self::QdrantSearch => "qdrant_search",
            Self::ReviewerExecution => "reviewer_execution",
            Self::ModelRequest => "model_request",
            Self::CandidateGenerated => "candidate_generated",
            Self::FindingVerification => "finding_verification",
            Self::Deduplication => "deduplication",
            Self::Publication => "publication",
        }
    }

    /// Parses a wire name.
    pub fn parse(name: &str) -> Option<SpanName> {
        Self::ALL.into_iter().find(|s| s.as_str() == name)
    }

    /// The parent in the expected trace tree.
    pub const fn parent(self) -> Parent {
        match self {
            Self::WebhookReceived => Parent::Root,
            Self::QdrantSearch => Parent::Span(Self::ContextSelection),
            Self::ModelRequest => Parent::Span(Self::ReviewerExecution),
            Self::CandidateGenerated => Parent::Span(Self::ModelRequest),
            Self::Publication => Parent::Job("review-publish"),
            Self::RepositoryIndex => Parent::Job("repository-index"),
            _ => Parent::Job("pr-review"),
        }
    }

    /// Attributes required beyond [`BASE_ATTRS`].
    pub const fn extra_attrs(self) -> &'static [&'static str] {
        match self {
            Self::WebhookReceived => &["event", "action", "delivery_id", "installation_id"],
            Self::RepositoryCheckout => &["commit_sha"],
            Self::IncrementalGraphUpdate => &["base_snapshot_id", "files_changed"],
            Self::ContextSelection => &["cluster_id", "reviewer_type", "token_budget"],
            Self::QdrantSearch => &["kinds", "limit", "hits"],
            Self::ReviewerExecution => &["reviewer_type", "version"],
            Self::ModelRequest => &["gen_ai.system", "gen_ai.request.model"],
            Self::CandidateGenerated => &["candidate_finding_id", "category", "severity"],
            Self::FindingVerification => &["candidate_finding_id", "outcome"],
            Self::Publication => &["pull_request_id", "findings_published"],
            _ => &[],
        }
    }

    /// Every required attribute: [`BASE_ATTRS`] then [`SpanName::extra_attrs`].
    pub fn required_attrs(self) -> Vec<&'static str> {
        BASE_ATTRS
            .iter()
            .chain(self.extra_attrs())
            .copied()
            .collect()
    }

    /// The task that owns the instrumentation (for the catalog document).
    pub const fn owner(self) -> &'static str {
        match self {
            Self::WebhookReceived => "GH-009 (apps/api webhooks)",
            Self::RepositoryCheckout => "PIPE-003",
            Self::RepositoryIndex => "IDX-001",
            Self::IncrementalGraphUpdate => "INC-001",
            Self::DiffAnalysis => "DIFF-001",
            Self::SymbolMapping => "DIFF-004",
            Self::ImpactAnalysis => "IMP-001",
            Self::ContextSelection => "CTX-001",
            Self::QdrantSearch => "SEM-005",
            Self::ReviewerExecution => "REV-001",
            Self::ModelRequest => "GW-010",
            Self::CandidateGenerated => "REV-001",
            Self::FindingVerification => "VER-001",
            Self::Deduplication => "DED-001",
            Self::Publication => "GH-010 (apps/api publisher)",
        }
    }
}

/// Whether `key` may appear on a span.
pub fn is_forbidden_attr(key: &str) -> bool {
    FORBIDDEN_ATTRS.contains(&key)
}

/// Creates an INFO span for a lifecycle stage. The name must be a [`SpanName`]:
///
/// ```
/// use telemetry::{stage_span, spans::SpanName};
/// let span = stage_span!(SpanName::ImpactAnalysis, organization_id = "o", review_run_id = "r");
/// drop(span);
/// ```
#[macro_export]
macro_rules! stage_span {
    ($name:expr $(, $($fields:tt)*)?) => {
        $crate::__private::tracing::info_span!(
            $crate::spans::SpanName::as_str($name)
            $(, $($fields)*)?
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for s in SpanName::ALL {
            assert_eq!(SpanName::parse(s.as_str()), Some(s));
            assert!(seen.insert(s.as_str()));
        }
        assert_eq!(seen.len(), 15);
    }

    #[test]
    fn every_span_requires_the_base_ids() {
        for s in SpanName::ALL {
            let attrs = s.required_attrs();
            for base in BASE_ATTRS {
                assert!(attrs.contains(&base), "{} lacks {base}", s.as_str());
            }
        }
    }
}
