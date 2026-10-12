import { randomUUID } from 'node:crypto';
import {
  Inject,
  Injectable,
  Logger,
  Optional,
  type BeforeApplicationShutdown,
} from '@nestjs/common';
import { SpanKind, trace } from '@opentelemetry/api';
import { sql } from 'kysely';
import type { Pool } from 'pg';
import type { JobCanceller } from '../common/job-canceller';
import { incCounter, recordHistogram, setGauge } from '../common/metrics';
import { DbService, PG_POOL } from '../db/db.module';
import type { Tx } from '../db/tx';
import { TRACER_NAME } from '../telemetry/tracer.service';
import type { PrReviewJob, ReviewJobs } from '../reviews/review-jobs.port';
import { JobConsumer } from './consumer';
import type {
  ActiveJob,
  CancelPredicate,
  ConsumeOptions,
  ConsumerHandle,
  EnqueueRequest,
  EnqueueResult,
  JobHandler,
  JobQueue,
} from './job-queue';
import {
  backoffMs,
  currentTraceParent,
  type ClaimedJob,
  type FailKind,
  type JobStore,
} from './job-store';
import { isJobQueueName, JOB_QUEUES, parsePayload, type JobQueueName } from './payloads';

/** Rate-limit re-queues refund the attempt; after this many the job is dead (PIPE-001). */
export const MAX_RATE_LIMIT_REQUEUES = 50;

const truncate = (text: string): string => (text.length > 2000 ? text.slice(0, 2000) : text);

/**
 * The PostgreSQL adapter of the job queue port over the shared `jobs` table (API-007).
 * Enqueue and cancel run in the caller transaction; claim, heartbeat, complete and fail run in
 * their own short transactions, without a tenant (the table is not tenant scoped).
 */
@Injectable()
export class PgJobQueue
  implements JobQueue, JobStore, JobCanceller, ReviewJobs, BeforeApplicationShutdown
{
  private readonly logger = new Logger(PgJobQueue.name);
  private readonly consumers = new Set<{ stop(graceMs?: number): Promise<void> }>();
  private lastDepthSample = 0;

  constructor(
    private readonly dbs: DbService,
    @Optional() @Inject(PG_POOL) private readonly pool?: Pool,
  ) {}

  async enqueue<Q extends JobQueueName>(
    trx: Tx,
    request: EnqueueRequest<Q>,
  ): Promise<EnqueueResult> {
    const payload = parsePayload(request.queue, request.payload);
    return trace
      .getTracer(TRACER_NAME)
      .startActiveSpan('job_enqueue', { kind: SpanKind.PRODUCER }, async (span) => {
        try {
          span.setAttribute('queue', request.queue);
          const id = randomUUID();
          const inserted = await trx
            .insertInto('jobs')
            .values({
              id,
              organization_id: request.organizationId,
              queue: request.queue,
              idempotency_key: request.idempotencyKey,
              payload: JSON.stringify(payload),
              priority: request.priority ?? 0,
              max_attempts: request.maxAttempts ?? 5,
              run_after: request.runAfter ?? sql<Date>`now()`,
              trace_parent: currentTraceParent(),
            })
            .onConflict((oc) => oc.column('idempotency_key').doNothing())
            .returning('id')
            .executeTakeFirst();
          if (!inserted) {
            const existing = await trx
              .selectFrom('jobs')
              .select('id')
              .where('idempotency_key', '=', request.idempotencyKey)
              .executeTakeFirstOrThrow();
            span.setAttribute('job_id', existing.id);
            span.setAttribute('created', false);
            return { jobId: existing.id, created: false };
          }
          // Same transaction: Postgres delivers the notification only when it commits.
          await sql`select pg_notify(${`jobs_${request.queue}`}, ${inserted.id})`.execute(trx);
          span.setAttribute('job_id', inserted.id);
          span.setAttribute('created', true);
          incCounter('jobs_enqueued_total', { queue: request.queue });
          return { jobId: inserted.id, created: true };
        } finally {
          span.end();
        }
      });
  }

  async cancelWhere(trx: Tx, predicate: CancelPredicate): Promise<number> {
    const { queue, reviewRunIds, repositoryIds, keyPrefix } = predicate;
    if (!queue && !reviewRunIds && !repositoryIds && keyPrefix === undefined) {
      throw new Error('cancelWhere needs at least one selector');
    }
    if (reviewRunIds?.length === 0 || repositoryIds?.length === 0) return 0;
    let q = trx.updateTable('jobs').set({ state: 'cancelled' }).where('state', '=', 'queued');
    if (queue) q = q.where('queue', '=', queue);
    if (reviewRunIds) {
      q = q.where(sql<boolean>`payload ->> 'review_run_id' = any(${reviewRunIds}::text[])`);
    }
    if (repositoryIds) {
      q = q.where(sql<boolean>`payload ->> 'repository_id' = any(${repositoryIds}::text[])`);
    }
    if (keyPrefix !== undefined) {
      q = q.where(sql<boolean>`starts_with(idempotency_key, ${keyPrefix})`);
    }
    const result = await q.executeTakeFirst();
    const count = Number(result.numUpdatedRows);
    if (count > 0) incCounter('jobs_cancelled_total', { queue: queue ?? 'any' }, count);
    return count;
  }

  cancelQueuedForRepositories(trx: Tx, repositoryIds: string[]): Promise<number> {
    return this.cancelWhere(trx, { repositoryIds });
  }

  /** SUP-001's `ReviewJobs`: the run's `pr-review` job, keyed like the run. */
  async enqueuePrReview(trx: Tx, job: PrReviewJob): Promise<{ created: boolean }> {
    const result = await this.enqueue(trx, {
      queue: 'pr-review',
      idempotencyKey: job.idempotencyKey,
      organizationId: job.organizationId,
      payload: { review_run_id: job.reviewRunId },
    });
    return { created: result.created };
  }

  cancelQueuedForRuns(trx: Tx, reviewRunIds: string[]): Promise<number> {
    return this.cancelWhere(trx, { reviewRunIds });
  }

  async findActive(trx: Tx, queue: JobQueueName, repositoryId: string): Promise<ActiveJob | null> {
    const row = await trx
      .selectFrom('jobs')
      .select(['id', 'queue', 'state'])
      .where('queue', '=', queue)
      .where('state', 'in', ['queued', 'running'])
      .where(sql<boolean>`payload ->> 'repository_id' = ${repositoryId}`)
      // A running job describes the state better than a queued one.
      .orderBy(sql`state = 'running'`, 'desc')
      .orderBy('created_at', 'desc')
      .executeTakeFirst();
    if (!row) return null;
    return { jobId: row.id, queue, state: row.state as ActiveJob['state'] };
  }

  consume<Q extends JobQueueName>(
    queue: Q,
    handler: JobHandler<Q>,
    options: ConsumeOptions = {},
  ): ConsumerHandle {
    const consumer = new JobConsumer(this, queue, handler, options, this.pool);
    this.consumers.add(consumer);
    consumer.start();
    return {
      stop: async (graceMs?: number) => {
        await consumer.stop(graceMs);
        this.consumers.delete(consumer);
      },
    };
  }

  /** Graceful release on shutdown: stop claiming, drain, re-queue what did not finish. */
  async beforeApplicationShutdown(): Promise<void> {
    await Promise.allSettled([...this.consumers].map((c) => c.stop()));
    this.consumers.clear();
  }

  // --- JobStore (worker side) ---

  /**
   * A worker transaction: no tenant, plus the transaction-local `app.job_worker` opt-in that the
   * `jobs` RLS policy requires for cross-tenant claims (SEC-001). Request paths never set it.
   */
  private workerTx<T>(fn: (trx: Tx) => Promise<T>): Promise<T> {
    return this.dbs.withTx(null, async (trx) => {
      await sql`select set_config('app.job_worker', 'on', true)`.execute(trx);
      return fn(trx);
    });
  }

  async claim(
    queues: readonly JobQueueName[],
    workerId: string,
    leaseMs: number,
  ): Promise<ClaimedJob | null> {
    const { rows } = await this.workerTx((trx) =>
      // The claim statement of target-architecture section 5 (shared with the Rust worker).
      sql<{
        id: string;
        queue: string;
        organization_id: string;
        payload: unknown;
        attempts: number;
        max_attempts: number;
        trace_parent: string | null;
        wait_seconds: number;
      }>`
        update jobs set state = 'running', locked_by = ${workerId},
               locked_until = now() + ${leaseMs}::float8 * interval '1 millisecond',
               attempts = attempts + 1
        where id = (select id from jobs
                    where queue = any(${[...queues]}::text[]) and state = 'queued'
                      and run_after <= now()
                    order by priority desc, created_at
                    for update skip locked limit 1)
        returning id, queue, organization_id, payload, attempts, max_attempts, trace_parent,
                  extract(epoch from now() - greatest(created_at, run_after))::float8
                    as wait_seconds`.execute(trx),
    );
    const row = rows[0];
    if (!row || !isJobQueueName(row.queue)) return null;
    recordHistogram('queue_wait_seconds', Math.max(0, row.wait_seconds), { queue: row.queue });
    return {
      id: row.id,
      queue: row.queue,
      organizationId: row.organization_id,
      payload: row.payload,
      attempts: row.attempts,
      maxAttempts: row.max_attempts,
      lockedBy: workerId,
      traceParent: row.trace_parent,
      waitSeconds: row.wait_seconds,
    };
  }

  async heartbeat(job: ClaimedJob, leaseMs: number): Promise<boolean> {
    const result = await this.workerTx((trx) =>
      trx
        .updateTable('jobs')
        .set({ locked_until: sql<Date>`now() + ${leaseMs}::float8 * interval '1 millisecond'` })
        .where('id', '=', job.id)
        .where('locked_by', '=', job.lockedBy)
        .where('attempts', '=', job.attempts)
        .where('state', '=', 'running')
        .executeTakeFirst(),
    );
    return result.numUpdatedRows > 0n;
  }

  async complete(job: ClaimedJob): Promise<boolean> {
    const result = await this.workerTx((trx) =>
      trx
        .updateTable('jobs')
        .set({ state: 'succeeded', locked_by: null, locked_until: null, last_error: null })
        .where('id', '=', job.id)
        .where('locked_by', '=', job.lockedBy)
        .where('attempts', '=', job.attempts)
        .where('state', '=', 'running')
        .executeTakeFirst(),
    );
    return result.numUpdatedRows > 0n;
  }

  /** Records a failure; returns the resulting state, or null when the lease was lost. */
  async fail(job: ClaimedJob, failure: FailKind): Promise<'queued' | 'dead' | null> {
    const { rows } = await this.workerTx((trx) => {
      if (failure.kind === 'rate_limited') {
        // The attempt is refunded: a rate limit says nothing about the job itself.
        return sql<{ state: 'queued' | 'dead' }>`
          update jobs set
            state = case when rate_limit_requeues + 1 >= ${MAX_RATE_LIMIT_REQUEUES}
                         then 'dead' else 'queued' end,
            attempts = greatest(attempts - 1, 0),
            rate_limit_requeues = rate_limit_requeues + 1,
            run_after = now() + ${Math.max(0, failure.retryAfterMs)}::float8 * interval '1 millisecond',
            locked_by = null, locked_until = null, last_error = 'rate_limited'
          where id = ${job.id} and locked_by = ${job.lockedBy} and attempts = ${job.attempts}
            and state = 'running'
          returning state`.execute(trx);
      }
      const permanent = failure.kind === 'permanent';
      const delay = backoffMs(job.attempts);
      return sql<{ state: 'queued' | 'dead' }>`
        update jobs set
          state = case when ${permanent} or attempts >= max_attempts then 'dead' else 'queued' end,
          run_after = case when ${permanent} or attempts >= max_attempts then run_after
                           else now() + ${delay}::float8 * interval '1 millisecond' end,
          locked_by = null, locked_until = null, last_error = ${truncate(failure.error)}
        where id = ${job.id} and locked_by = ${job.lockedBy} and attempts = ${job.attempts}
          and state = 'running'
        returning state`.execute(trx);
    });
    const state = rows[0]?.state ?? null;
    if (state) incCounter('jobs_failed_total', { queue: job.queue, kind: failure.kind });
    if (state === 'dead') {
      incCounter('jobs_dead_total', { queue: job.queue });
      this.logger.warn(`job ${job.id} (${job.queue}) is dead after ${job.attempts} attempts`);
    }
    return state;
  }

  /** Returns this worker's running jobs to `queued`, refunding the attempt. */
  async release(workerId: string, jobIds?: string[]): Promise<number> {
    if (jobIds?.length === 0) return 0;
    const result = await this.workerTx((trx) => {
      let q = trx
        .updateTable('jobs')
        .set({
          state: 'queued',
          locked_by: null,
          locked_until: null,
          attempts: sql<number>`greatest(attempts - 1, 0)`,
          last_error: 'released_on_shutdown',
        })
        .where('locked_by', '=', workerId)
        .where('state', '=', 'running');
      if (jobIds) q = q.where('id', 'in', jobIds);
      return q.executeTakeFirst();
    });
    return Number(result.numUpdatedRows);
  }

  /** Samples `queue_depth{queue}` (queued jobs), at most every 30 s. */
  async sampleDepth(force = false): Promise<void> {
    const now = Date.now();
    if (!force && now - this.lastDepthSample < 30_000) return;
    this.lastDepthSample = now;
    const rows = await this.workerTx((trx) =>
      trx
        .selectFrom('jobs')
        .select(['queue', (eb) => eb.fn.countAll<string>().as('n')])
        .where('state', '=', 'queued')
        .groupBy('queue')
        .execute(),
    );
    const counts = new Map(rows.map((r) => [r.queue, Number(r.n)]));
    for (const queue of JOB_QUEUES) setGauge('queue_depth', counts.get(queue) ?? 0, { queue });
  }
}
