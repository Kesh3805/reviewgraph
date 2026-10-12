import { Injectable } from '@nestjs/common';

/** What the broker needs to know about the job a clone credential is requested for (SEC-005). */
export interface CloneJobState {
  state: 'queued' | 'running' | 'succeeded' | 'failed' | 'dead' | 'cancelled' | string;
  /** The repository the job works on, or null when the job is not repository-scoped. */
  repositoryId: string | null;
}

/**
 * Looks up the job named by the service token's `job` claim. `null` means the job does not exist
 * (answered 404, no oracle); `undefined` means the lookup is not available in this deployment.
 */
export interface CloneJobGate {
  lookup(jobId: string): Promise<CloneJobState | null | undefined>;
}

export const CLONE_JOB_GATE = Symbol('CLONE_JOB_GATE');

/**
 * Default until the Postgres job queue adapter (API-007) provides a lookup over the `jobs` table:
 * the job check is skipped and the repository claim alone scopes the credential.
 */
@Injectable()
export class UnwiredCloneJobGate implements CloneJobGate {
  lookup(): Promise<CloneJobState | null | undefined> {
    return Promise.resolve(undefined);
  }
}

export type JobCheck = 'ok' | 'skipped' | 'not_found' | 'not_running';

/** The broker's decision for a job claim against the requested repository. */
export async function checkCloneJob(
  gate: CloneJobGate,
  jobId: string | undefined,
  repositoryId: string,
): Promise<JobCheck> {
  if (!jobId) return 'skipped';
  const job = await gate.lookup(jobId);
  if (job === undefined) return 'skipped';
  // A job for another repository is indistinguishable from a missing one.
  if (job === null || (job.repositoryId !== null && job.repositoryId !== repositoryId)) {
    return 'not_found';
  }
  return job.state === 'running' ? 'ok' : 'not_running';
}
