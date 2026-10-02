//! Standard correlation attribute names (target-architecture §8) and a typed helper that
//! stamps them onto spans.

use review_core::ids::{CandidateFindingId, CommitSha, OrganizationId, PullRequestId};
use review_core::ids::{RepositoryId, ReviewRunId};
use review_core::reviewer_type::ReviewerType;

pub const REQUEST_ID: &str = "request_id";
pub const REVIEW_RUN_ID: &str = "review_run_id";
pub const REPOSITORY_ID: &str = "repository_id";
pub const ORGANIZATION_ID: &str = "organization_id";
pub const PULL_REQUEST_ID: &str = "pull_request_id";
pub const COMMIT_SHA: &str = "commit_sha";
pub const JOB_ID: &str = "job_id";
pub const REVIEWER_TYPE: &str = "reviewer_type";
pub const CANDIDATE_FINDING_ID: &str = "candidate_finding_id";

/// Every correlation attribute, in a stable order. JSON log lines flatten exactly these keys.
pub const CORRELATION_KEYS: [&str; 9] = [
    REQUEST_ID,
    REVIEW_RUN_ID,
    REPOSITORY_ID,
    ORGANIZATION_ID,
    PULL_REQUEST_ID,
    COMMIT_SHA,
    JOB_ID,
    REVIEWER_TYPE,
    CANDIDATE_FINDING_ID,
];

/// Correlation attributes for a span. Unset fields are left out of the span.
#[derive(Debug, Clone, Default)]
pub struct Correlation {
    pub request_id: Option<String>,
    pub review_run_id: Option<ReviewRunId>,
    pub repository_id: Option<RepositoryId>,
    pub organization_id: Option<OrganizationId>,
    pub pull_request_id: Option<PullRequestId>,
    pub commit_sha: Option<CommitSha>,
    pub job_id: Option<String>,
    pub reviewer_type: Option<ReviewerType>,
    pub candidate_finding_id: Option<CandidateFindingId>,
}

impl Correlation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_id(mut self, v: impl Into<String>) -> Self {
        self.request_id = Some(v.into());
        self
    }

    pub fn review_run_id(mut self, v: ReviewRunId) -> Self {
        self.review_run_id = Some(v);
        self
    }

    pub fn repository_id(mut self, v: RepositoryId) -> Self {
        self.repository_id = Some(v);
        self
    }

    pub fn organization_id(mut self, v: OrganizationId) -> Self {
        self.organization_id = Some(v);
        self
    }

    pub fn pull_request_id(mut self, v: PullRequestId) -> Self {
        self.pull_request_id = Some(v);
        self
    }

    pub fn commit_sha(mut self, v: CommitSha) -> Self {
        self.commit_sha = Some(v);
        self
    }

    pub fn job_id(mut self, v: impl Into<String>) -> Self {
        self.job_id = Some(v.into());
        self
    }

    pub fn reviewer_type(mut self, v: ReviewerType) -> Self {
        self.reviewer_type = Some(v);
        self
    }

    pub fn candidate_finding_id(mut self, v: CandidateFindingId) -> Self {
        self.candidate_finding_id = Some(v);
        self
    }

    /// Records every set attribute onto `span`. The span must have been created with the
    /// [`correlation_span!`](crate::correlation_span) macro (which declares the fields).
    pub fn record(&self, span: &tracing::Span) {
        fn put<T: std::fmt::Display>(span: &tracing::Span, key: &'static str, v: &Option<T>) {
            if let Some(v) = v {
                span.record(key, tracing::field::display(v));
            }
        }
        put(span, REQUEST_ID, &self.request_id);
        put(span, REVIEW_RUN_ID, &self.review_run_id);
        put(span, REPOSITORY_ID, &self.repository_id);
        put(span, ORGANIZATION_ID, &self.organization_id);
        put(span, PULL_REQUEST_ID, &self.pull_request_id);
        put(span, COMMIT_SHA, &self.commit_sha);
        put(span, JOB_ID, &self.job_id);
        put(span, REVIEWER_TYPE, &self.reviewer_type);
        put(span, CANDIDATE_FINDING_ID, &self.candidate_finding_id);
    }
}

/// Creates an `info` span carrying the standard correlation attributes.
///
/// ```
/// use telemetry::{correlation_span, attrs::Correlation};
/// use review_core::ids::ReviewRunId;
/// let corr = Correlation::new().review_run_id(ReviewRunId::new());
/// let span = correlation_span!("diff_analysis", &corr);
/// let _entered = span.enter();
/// ```
#[macro_export]
macro_rules! correlation_span {
    ($name:literal, $corr:expr) => {{
        let span = $crate::__private::tracing::info_span!(
            $name,
            request_id = $crate::__private::tracing::field::Empty,
            review_run_id = $crate::__private::tracing::field::Empty,
            repository_id = $crate::__private::tracing::field::Empty,
            organization_id = $crate::__private::tracing::field::Empty,
            pull_request_id = $crate::__private::tracing::field::Empty,
            commit_sha = $crate::__private::tracing::field::Empty,
            job_id = $crate::__private::tracing::field::Empty,
            reviewer_type = $crate::__private::tracing::field::Empty,
            candidate_finding_id = $crate::__private::tracing::field::Empty,
        );
        $crate::attrs::Correlation::record($corr, &span);
        span
    }};
}
