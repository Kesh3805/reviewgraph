import { Global, Injectable, Module, ServiceUnavailableException } from '@nestjs/common';
import type { Tx } from '../db/tx';

/**
 * The slice of the job queue the control plane needs to start indexing (API-008). The Postgres
 * adapter over the shared `jobs` table (transactional enqueue, `ON CONFLICT (idempotency_key)`)
 * is API-007; payloads carry ids only.
 */
export type JobQueueName = 'repository-index';

export interface RepositoryIndexPayload {
  repository_id: string;
  organization_id: string;
  /** The default-branch head the index is built for. */
  head_sha: string;
  /** True for a forced full index (graph rebuild). */
  force: boolean;
}

export interface EnqueueRequest {
  queue: JobQueueName;
  idempotencyKey: string;
  payload: RepositoryIndexPayload;
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

export interface JobQueue {
  /** Enqueues in the caller transaction, so the job exists only if the state change commits. */
  enqueue(trx: Tx, request: EnqueueRequest): Promise<EnqueueResult>;
  /** The queued or running job of a repository, if any. */
  findActive(trx: Tx, queue: JobQueueName, repositoryId: string): Promise<ActiveJob | null>;
}
export const JOB_QUEUE = Symbol('JOB_QUEUE');

/** Default until API-007 provides the adapter: starting an index answers 503, reads show none. */
@Injectable()
export class UnwiredJobQueue implements JobQueue {
  enqueue(): Promise<EnqueueResult> {
    return Promise.reject(new ServiceUnavailableException('the job queue is not available yet'));
  }

  findActive(): Promise<ActiveJob | null> {
    return Promise.resolve(null);
  }
}

@Global()
@Module({
  providers: [{ provide: JOB_QUEUE, useClass: UnwiredJobQueue }],
  exports: [JOB_QUEUE],
})
export class JobsModule {}
