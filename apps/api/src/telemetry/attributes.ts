/**
 * Standard span attribute names. These mirror the Rust constants (OBS-001) and
 * target-architecture section 8; keep the two lists identical.
 */
export const Attr = {
  RequestId: 'request_id',
  ReviewRunId: 'review_run_id',
  RepositoryId: 'repository_id',
  OrganizationId: 'organization_id',
  PullRequestId: 'pull_request_id',
  CommitSha: 'commit_sha',
  JobId: 'job_id',
  ReviewerType: 'reviewer_type',
  CandidateFindingId: 'candidate_finding_id',
  /** Set from `Classify::class()` on failure (error taxonomy). */
  ErrorClass: 'error.class',
} as const;

export type AttrName = (typeof Attr)[keyof typeof Attr];
export type SpanAttributes = Partial<Record<AttrName, string | number | boolean>>;

/** The correlation attributes copied onto log lines from the active span. */
export const CORRELATION_ATTRIBUTES: readonly AttrName[] = [
  Attr.RequestId,
  Attr.ReviewRunId,
  Attr.RepositoryId,
  Attr.OrganizationId,
  Attr.PullRequestId,
  Attr.CommitSha,
  Attr.JobId,
  Attr.ReviewerType,
  Attr.CandidateFindingId,
];

/** The only stage span names (target-architecture section 8). */
export const SPAN_NAMES = [
  'webhook_received',
  'repository_checkout',
  'repository_index',
  'incremental_graph_update',
  'diff_analysis',
  'symbol_mapping',
  'impact_analysis',
  'context_selection',
  'qdrant_search',
  'reviewer_execution',
  'model_request',
  'candidate_generated',
  'finding_verification',
  'deduplication',
  'publication',
] as const;

export type StageSpanName = (typeof SPAN_NAMES)[number];
