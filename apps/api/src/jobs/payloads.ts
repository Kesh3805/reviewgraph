import { z } from 'zod';

/**
 * Job payloads of the shared queue (ADR-012, PRD sections 75 and 76). Payloads carry ids only:
 * no source, no secrets and no free text, so every string field is a uuid or a commit sha.
 * They mirror the Rust payload structs of the pipeline crate; `jobs.payload` is limited to 16 KiB.
 */
export const JOB_QUEUES = [
  'repository-index',
  'incremental-index',
  'pr-review',
  'review-publish',
  'history-ingest',
] as const;

export type JobQueueName = (typeof JOB_QUEUES)[number];

const uuid = z.uuid();
const sha = z.string().regex(/^([0-9a-f]{40}|[0-9a-f]{64})$/);

export const RepositoryIndexPayloadSchema = z
  .object({
    repository_id: uuid,
    organization_id: uuid,
    /** The default-branch head the index is built for. */
    head_sha: sha,
    /** True for a forced full index (graph rebuild). */
    force: z.boolean(),
  })
  .strict();

export const IncrementalIndexPayloadSchema = z
  .object({ repository_id: uuid, organization_id: uuid, head_sha: sha })
  .strict();

export const PrReviewPayloadSchema = z.object({ review_run_id: uuid }).strict();

export const ReviewPublishPayloadSchema = z.object({ review_run_id: uuid }).strict();

export const HistoryIngestPayloadSchema = z
  .object({ repository_id: uuid, organization_id: uuid })
  .strict();

export const JOB_PAYLOAD_SCHEMAS = {
  'repository-index': RepositoryIndexPayloadSchema,
  'incremental-index': IncrementalIndexPayloadSchema,
  'pr-review': PrReviewPayloadSchema,
  'review-publish': ReviewPublishPayloadSchema,
  'history-ingest': HistoryIngestPayloadSchema,
} as const satisfies Record<JobQueueName, z.ZodType>;

export interface JobPayloads {
  'repository-index': z.infer<typeof RepositoryIndexPayloadSchema>;
  'incremental-index': z.infer<typeof IncrementalIndexPayloadSchema>;
  'pr-review': z.infer<typeof PrReviewPayloadSchema>;
  'review-publish': z.infer<typeof ReviewPublishPayloadSchema>;
  'history-ingest': z.infer<typeof HistoryIngestPayloadSchema>;
}

export type RepositoryIndexPayload = JobPayloads['repository-index'];

/** Validates a payload for its queue; throws a ZodError for anything else. */
export function parsePayload<Q extends JobQueueName>(queue: Q, payload: unknown): JobPayloads[Q] {
  return JOB_PAYLOAD_SCHEMAS[queue].parse(payload) as JobPayloads[Q];
}

export function isJobQueueName(value: string): value is JobQueueName {
  return (JOB_QUEUES as readonly string[]).includes(value);
}

/** Idempotency keys (PRD section 76). */
export const jobKeys = {
  repositoryIndex: (repositoryId: string, headSha: string): string =>
    `repo-index:${repositoryId}:${headSha}`,
  prReview: (provider: string, repositoryId: string, prNumber: number, headSha: string): string =>
    `pr-review:${provider}:${repositoryId}:${prNumber}:${headSha}`,
  publish: (reviewRunId: string): string => `publish:${reviewRunId}`,
};
