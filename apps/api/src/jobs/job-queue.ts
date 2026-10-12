import type { Tx } from '../db/tx';
import type { JobPayloads, JobQueueName } from './payloads';

export type { JobPayloads, JobQueueName, RepositoryIndexPayload } from './payloads';

/**
 * The TypeScript side of the shared job queue port (ADR-012, API-007). The Rust side
 * (`pipeline::jobs`, PIPE-001) implements the same contract over the same `jobs` table:
 * transactional enqueue with `ON CONFLICT (idempotency_key) DO NOTHING`, a `pg_notify` wake-up
 * delivered on commit, and the SKIP LOCKED claim of target-architecture section 5.
 */
export interface EnqueueRequest<Q extends JobQueueName = JobQueueName> {
  queue: Q;
  idempotencyKey: string;
  /** The tenant the job works for (`jobs.organization_id`). */
  organizationId: string;
  payload: JobPayloads[Q];
  /** Higher first. Default 0. */
  priority?: number;
  /** 1..=20, default 5. */
  maxAttempts?: number;
  /** Not claimable before this instant. Default now. */
  runAfter?: Date;
}

export interface EnqueueResult {
  /** The new job, or the existing one when the idempotency key was already taken. */
  jobId: string;
  created: boolean;
}

export interface ActiveJob {
  jobId: string;
  queue: JobQueueName;
  state: 'queued' | 'running';
}

/** Which queued jobs to cancel. At least one selector is required; selectors combine with AND. */
export interface CancelPredicate {
  queue?: JobQueueName;
  /** Jobs whose payload names one of these review runs. */
  reviewRunIds?: string[];
  /** Jobs whose payload names one of these repositories. */
  repositoryIds?: string[];
  /** Jobs whose idempotency key starts with this prefix. */
  keyPrefix?: string;
}

/** A claimed job as a handler sees it. */
export interface JobContext<Q extends JobQueueName = JobQueueName> {
  jobId: string;
  queue: Q;
  organizationId: string;
  payload: JobPayloads[Q];
  /** 1 on the first claim. */
  attempt: number;
  maxAttempts: number;
  traceParent: string | null;
  /** Aborted when the lease is lost or the consumer shuts down; the handler should stop. */
  signal: AbortSignal;
}

export type JobHandler<Q extends JobQueueName = JobQueueName> = (
  job: JobContext<Q>,
) => Promise<void>;

export interface ConsumeOptions {
  /** Handlers running at once. Default from config (`review-publish: 4`). */
  concurrency?: number;
  /** Lease length; the heartbeat runs every lease/3. Default 60 s. */
  leaseMs?: number;
  /** Idle poll interval (picks up delayed jobs and lost notifications). Default 5 s. */
  pollMs?: number;
}

export interface ConsumerHandle {
  /**
   * Stops claiming, waits up to `graceMs` (default 20 s) for running handlers, then returns
   * unfinished jobs to `queued` with `locked_by = NULL`.
   */
  stop(graceMs?: number): Promise<void>;
}

export interface JobQueue {
  /** Enqueues in the caller transaction, so the job exists only if the state change commits. */
  enqueue<Q extends JobQueueName>(trx: Tx, request: EnqueueRequest<Q>): Promise<EnqueueResult>;
  /** Cancels matching `queued` jobs (running ones observe their own cancellation). */
  cancelWhere(trx: Tx, predicate: CancelPredicate): Promise<number>;
  /** The queued or running job of a repository, if any. */
  findActive(trx: Tx, queue: JobQueueName, repositoryId: string): Promise<ActiveJob | null>;
  /** Starts consuming a queue in this process. */
  consume<Q extends JobQueueName>(
    queue: Q,
    handler: JobHandler<Q>,
    options?: ConsumeOptions,
  ): ConsumerHandle;
}

export const JOB_QUEUE = Symbol('JOB_QUEUE');

/** Thrown by a handler for a failure that retrying cannot fix: the job goes `dead` at once. */
export class PermanentJobError extends Error {
  constructor(message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.name = 'PermanentJobError';
  }
}
