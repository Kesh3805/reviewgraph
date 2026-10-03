import { Injectable } from '@nestjs/common';
import type { Tx } from '../db/tx';

/**
 * Cancels queued jobs of repositories whose access was revoked (GH-013):
 * `UPDATE jobs SET state='cancelled' WHERE state='queued' AND payload->>'repository_id' = ANY($ids)`.
 * The `jobs` table and its TS adapter arrive with API-007, which replaces the no-op below.
 * It runs inside the caller's transaction so the cancellation commits with the lifecycle change.
 */
export interface JobCanceller {
  /** Returns how many queued jobs were cancelled. */
  cancelQueuedForRepositories(trx: Tx, repositoryIds: string[]): Promise<number>;
}
export const JOB_CANCELLER = Symbol('JOB_CANCELLER');

/** Placeholder until API-007 provides the real queue adapter: there is no queue to cancel yet. */
@Injectable()
export class NoopJobCanceller implements JobCanceller {
  cancelQueuedForRepositories(): Promise<number> {
    return Promise.resolve(0);
  }
}
