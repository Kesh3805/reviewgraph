import { Injectable, ServiceUnavailableException } from '@nestjs/common';
import type { Tx } from '../db/tx';

/**
 * The slice of the shared job queue the review orchestration needs (SUP-001): enqueue the
 * `pr-review` job of a new run and cancel the queued jobs of superseded or cancelled runs, both
 * in the caller's transaction (ADR-012), so they commit or roll back with the state change.
 * The adapter over the `jobs` table is API-007's `JobQueue`; payloads carry ids only.
 */
export interface PrReviewJob {
  reviewRunId: string;
  organizationId: string;
  repositoryId: string;
  /** Equal to the run's idempotency key: `pr-review:{provider}:{repo}:{pr}:{head}[:{suffix}]`. */
  idempotencyKey: string;
}

export interface ReviewJobs {
  enqueuePrReview(trx: Tx, job: PrReviewJob): Promise<{ created: boolean }>;
  /**
   * `UPDATE jobs SET state='cancelled' WHERE state='queued' AND payload->>'review_run_id' = ANY($1)`.
   * Running jobs are never killed: they observe the run state at their next stage boundary.
   */
  cancelQueuedForRuns(trx: Tx, reviewRunIds: string[]): Promise<number>;
}
export const REVIEW_JOBS = Symbol('REVIEW_JOBS');

/**
 * Default until the queue adapter (API-007) is wired: starting a review fails (and rolls back,
 * so no run exists without its job); there is nothing queued to cancel.
 */
@Injectable()
export class UnwiredReviewJobs implements ReviewJobs {
  enqueuePrReview(): Promise<{ created: boolean }> {
    return Promise.reject(new ServiceUnavailableException('the job queue is not available yet'));
  }

  cancelQueuedForRuns(): Promise<number> {
    return Promise.resolve(0);
  }
}
