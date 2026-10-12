import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';

/** `review_runs.state` (mirrors review_core::review::ReviewState). */
export const REVIEW_STATES = [
  'RECEIVED',
  'INDEXING',
  'ANALYZING',
  'REVIEWING',
  'VERIFYING',
  'PUBLISHING',
  'COMPLETED',
  'FAILED_INDEXING',
  'FAILED_ANALYSIS',
  'FAILED_REVIEW',
  'FAILED_PUBLISH',
  'SUPERSEDED',
  'CANCELLED',
] as const;
export type ReviewState = (typeof REVIEW_STATES)[number];

/** Non-terminal states: at most one run per pull request is in one of these. */
export const ACTIVE_REVIEW_STATES = [
  'RECEIVED',
  'INDEXING',
  'ANALYZING',
  'REVIEWING',
  'VERIFYING',
  'PUBLISHING',
] as const satisfies readonly ReviewState[];

export const isActiveState = (state: string): boolean =>
  (ACTIVE_REVIEW_STATES as readonly string[]).includes(state);

const uuid = z.string().uuid();
const timestamp = z.string();
const nullableTimestamp = z.string().nullable();

export const ReviewSummarySchema = z.object({
  id: uuid,
  pull_request_id: uuid,
  repository_id: uuid,
  state: z.enum(REVIEW_STATES),
  trigger: z.enum(['webhook', 'manual', 'reconciler', 'cli']),
  base_sha: z.string(),
  head_sha: z.string(),
  superseded_by: uuid.nullable(),
  retry_of: uuid.nullable(),
  degraded_reviewers: z.array(z.string()),
  created_at: timestamp,
  updated_at: timestamp,
  completed_at: nullableTimestamp,
});

export const PullRequestSchema = z.object({
  id: uuid,
  repository_id: uuid,
  number: z.number().int(),
  title: z.string(),
  author_login: z.string(),
  base_ref: z.string(),
  head_ref: z.string(),
  base_sha: z.string(),
  head_sha: z.string(),
  state: z.enum(['open', 'closed', 'merged']),
  draft: z.boolean(),
  created_at: timestamp,
  updated_at: timestamp,
  /** The most recent review run of the pull request, if any. */
  latest_review: z
    .object({ id: uuid, state: z.enum(REVIEW_STATES), head_sha: z.string(), created_at: timestamp })
    .nullable(),
});

const page = <T extends z.ZodType>(item: T) =>
  z.object({
    items: z.array(item),
    /** Opaque; pass it back as `cursor` for the next page. Null on the last page. */
    next_cursor: z.string().nullable(),
  });

export const PullRequestListSchema = page(PullRequestSchema);
export const ReviewListSchema = page(ReviewSummarySchema);

export const ListPullRequestsQuerySchema = z.object({
  state: z.enum(['open', 'closed', 'all']).default('open'),
  limit: z.coerce.number().int().min(1).max(100).default(50),
  cursor: z.string().max(500).optional(),
});

export const ListReviewsQuerySchema = z.object({
  limit: z.coerce.number().int().min(1).max(100).default(50),
  cursor: z.string().max(500).optional(),
});

const ReviewerRunSchema = z.object({
  id: uuid,
  reviewer: z.string(),
  cluster_key: z.string().nullable(),
  state: z.enum(['pending', 'running', 'succeeded', 'failed', 'skipped', 'timed_out']),
  error_class: z.string().nullable(),
  provider: z.string().nullable(),
  model: z.string().nullable(),
  prompt_version: z.string().nullable(),
  reviewer_version: z.string().nullable(),
  input_tokens: z.number().int(),
  output_tokens: z.number().int(),
  cost_usd_micros: z.number().int(),
  latency_ms: z.number().int().nullable(),
  started_at: nullableTimestamp,
  finished_at: nullableTimestamp,
});

/** Computed from `reviewer_runs` rows, never self-reported (INV-013). */
export const CompletenessSchema = z.object({
  reviewers_planned: z.number().int(),
  reviewers_succeeded: z.number().int(),
  reviewers_failed: z.array(z.object({ reviewer: z.string(), error_class: z.string().nullable() })),
  /** Reviewers that were planned but did not run (skipped, or never started). */
  not_executed: z.array(z.string()),
});

export const ReviewDetailSchema = ReviewSummarySchema.extend({
  failure: z.object({ class: z.string(), detail: z.string().nullable() }).nullable(),
  /** Stage timings (from the run's state transitions; empty until they are recorded). */
  stages: z.array(
    z.object({
      state: z.string(),
      entered_at: timestamp,
      duration_ms: z.number().nullable(),
    }),
  ),
  reviewer_runs: z.array(ReviewerRunSchema),
  completeness: CompletenessSchema,
  degraded: z.object({
    degraded: z.boolean(),
    reviewers: z.array(z.string()),
    reasons: z.array(z.object({ reviewer: z.string(), error_class: z.string().nullable() })),
  }),
  /** Risk assessment, change summary and coverage as recorded by the engine (null until then). */
  risk_assessment: z.unknown().nullable(),
  change_summary: z.unknown().nullable(),
  coverage: z
    .object({
      reviewed_clusters: z.array(z.string()),
      unreviewed_clusters: z.array(z.string()),
    })
    .nullable(),
  /** Candidate findings per lifecycle state, plus the published count. */
  finding_counts: z.object({
    by_state: z.record(z.string(), z.number().int()),
    published: z.number().int(),
  }),
  /** The W3C trace id, so the UI can deep-link to the trace. */
  trace_id: z.string().nullable(),
});

export const StartReviewResultSchema = z.object({
  review_run_id: uuid,
  job_id: z.string().nullable(),
  created: z.boolean(),
  head_sha: z.string(),
  superseded_run_ids: z.array(uuid),
});

export const CancelReviewResultSchema = z.object({
  review_run_id: uuid,
  state: z.enum(REVIEW_STATES),
  cancelled_jobs: z.number().int(),
});

export class PullRequestDto extends createZodDto(PullRequestSchema) {}
export class PullRequestListDto extends createZodDto(PullRequestListSchema) {}
export class ReviewListDto extends createZodDto(ReviewListSchema) {}
export class ReviewDetailDto extends createZodDto(ReviewDetailSchema) {}
export class StartReviewResultDto extends createZodDto(StartReviewResultSchema) {}
export class CancelReviewResultDto extends createZodDto(CancelReviewResultSchema) {}
export class ListPullRequestsQueryDto extends createZodDto(ListPullRequestsQuerySchema) {}
export class ListReviewsQueryDto extends createZodDto(ListReviewsQuerySchema) {}

export type PullRequestResponse = z.infer<typeof PullRequestSchema>;
export type ReviewSummaryResponse = z.infer<typeof ReviewSummarySchema>;
export type ReviewDetailResponse = z.infer<typeof ReviewDetailSchema>;
export type StartReviewResult = z.infer<typeof StartReviewResultSchema>;
export type CancelReviewResult = z.infer<typeof CancelReviewResultSchema>;
export type Completeness = z.infer<typeof CompletenessSchema>;
