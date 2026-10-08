import type { Tx } from '../db/tx';

/**
 * Cancels queued jobs of repositories whose access was revoked (GH-013):
 * `UPDATE jobs SET state='cancelled' WHERE state='queued' AND payload->>'repository_id' = ANY($ids)`.
 * Implemented by the queue adapter (`PgJobQueue`, API-007). It runs inside the caller's
 * transaction so the cancellation commits with the lifecycle change.
 */
export interface JobCanceller {
  /** Returns how many queued jobs were cancelled. */
  cancelQueuedForRepositories(trx: Tx, repositoryIds: string[]): Promise<number>;
}
export const JOB_CANCELLER = Symbol('JOB_CANCELLER');
