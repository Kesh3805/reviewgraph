import { randomUUID } from 'node:crypto';
import { hostname } from 'node:os';
import { Logger } from '@nestjs/common';
import { SpanKind, SpanStatusCode, trace } from '@opentelemetry/api';
import type { Pool, PoolClient } from 'pg';
import { incCounter, recordHistogram } from '../common/metrics';
import { ProviderError } from '../providers/ports';
import { TRACER_NAME } from '../telemetry/tracer.service';
import {
  PermanentJobError,
  type ConsumeOptions,
  type JobContext,
  type JobHandler,
} from './job-queue';
import { linkFromTraceParent, type ClaimedJob, type FailKind, type JobStore } from './job-store';
import { parsePayload, type JobQueueName } from './payloads';

/** Per-queue handler concurrency (target-architecture section 5: `review-publish: 4`). */
export const DEFAULT_CONCURRENCY: Record<JobQueueName, number> = {
  'repository-index': 1,
  'incremental-index': 1,
  'pr-review': 1,
  'review-publish': 4,
  'history-ingest': 1,
};
export const DEFAULT_LEASE_MS = 60_000;
export const DEFAULT_POLL_MS = 5_000;
export const SHUTDOWN_GRACE_MS = 20_000;

interface InFlight {
  job: ClaimedJob;
  controller: AbortController;
  done: Promise<void>;
}

/**
 * Consumes one queue: LISTEN `jobs_<queue>` for wake-ups, claim with SKIP LOCKED, run up to
 * `concurrency` handlers, heartbeat every lease/3, complete or fail with backoff. Notifications
 * are hints only: an idle poll (with jitter) picks up delayed jobs and lost notifications.
 */
export class JobConsumer<Q extends JobQueueName = JobQueueName> {
  private readonly logger = new Logger(JobConsumer.name);
  readonly workerId = `rg-api:${hostname()}:${process.pid}:${randomUUID().slice(0, 8)}`;
  private readonly concurrency: number;
  private readonly leaseMs: number;
  private readonly pollMs: number;
  private readonly inflight = new Map<string, InFlight>();
  private readonly released = new Set<string>();
  private running = false;
  private loop: Promise<void> | null = null;
  private wakePending = false;
  private wakeWaiter: (() => void) | null = null;
  private listener: PoolClient | null = null;
  private listenerRetry: NodeJS.Timeout | null = null;
  private stopping: Promise<void> | null = null;

  constructor(
    private readonly store: JobStore,
    readonly queue: Q,
    private readonly handler: JobHandler<Q>,
    options: ConsumeOptions = {},
    private readonly pool?: Pool,
  ) {
    this.concurrency = Math.max(1, options.concurrency ?? DEFAULT_CONCURRENCY[queue]);
    this.leaseMs = Math.max(300, options.leaseMs ?? DEFAULT_LEASE_MS);
    this.pollMs = Math.max(10, options.pollMs ?? DEFAULT_POLL_MS);
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    void this.listen();
    this.loop = this.run();
  }

  /** Wakes the claim loop (a notification, a finished handler or a test). */
  wake(): void {
    if (this.wakeWaiter) {
      const resolve = this.wakeWaiter;
      this.wakeWaiter = null;
      resolve();
    } else {
      this.wakePending = true;
    }
  }

  get activeJobIds(): string[] {
    return [...this.inflight.keys()];
  }

  stop(graceMs = SHUTDOWN_GRACE_MS): Promise<void> {
    this.stopping ??= this.shutdown(graceMs);
    return this.stopping;
  }

  private async shutdown(graceMs: number): Promise<void> {
    this.running = false;
    if (this.listenerRetry) clearTimeout(this.listenerRetry);
    this.wake();
    await this.loop;
    if (this.inflight.size > 0) {
      let timer: NodeJS.Timeout | undefined;
      await Promise.race([
        Promise.allSettled([...this.inflight.values()].map((f) => f.done)),
        new Promise<void>((resolve) => {
          timer = setTimeout(resolve, graceMs);
        }),
      ]);
      if (timer) clearTimeout(timer);
    }
    const unfinished = [...this.inflight.values()];
    if (unfinished.length > 0) {
      for (const flight of unfinished) {
        this.released.add(flight.job.id);
        flight.controller.abort(new Error('shutdown'));
      }
      try {
        const count = await this.store.release(
          this.workerId,
          unfinished.map((f) => f.job.id),
        );
        this.logger.warn(`released ${count} unfinished ${this.queue} job(s) on shutdown`);
      } catch (err) {
        // The reaper re-queues them once the lease expires.
        this.logger.error(`releasing jobs on shutdown failed: ${String(err)}`);
      }
    }
    await this.closeListener();
  }

  private async run(): Promise<void> {
    while (this.running) {
      if (this.inflight.size >= this.concurrency) {
        await this.waitForWake(this.pollMs);
        continue;
      }
      let job: ClaimedJob | null;
      try {
        job = await this.store.claim([this.queue], this.workerId, this.leaseMs);
      } catch (err) {
        this.logger.error(`claim on ${this.queue} failed: ${String(err)}`);
        await this.waitForWake(this.pollMs);
        continue;
      }
      if (!job) {
        await this.store.sampleDepth().catch(() => undefined);
        // Up to 20 % jitter so idle consumers do not poll in lockstep.
        await this.waitForWake(this.pollMs + Math.floor(Math.random() * this.pollMs * 0.2));
        continue;
      }
      if (!this.running) {
        // Claimed while stopping: hand it straight back.
        await this.store.release(this.workerId, [job.id]).catch(() => undefined);
        break;
      }
      this.dispatch(job);
    }
  }

  private waitForWake(ms: number): Promise<void> {
    if (this.wakePending || !this.running) {
      this.wakePending = false;
      return Promise.resolve();
    }
    return new Promise<void>((resolve) => {
      const timer = setTimeout(() => {
        this.wakeWaiter = null;
        resolve();
      }, ms);
      this.wakeWaiter = () => {
        clearTimeout(timer);
        resolve();
      };
    });
  }

  private dispatch(job: ClaimedJob): void {
    const controller = new AbortController();
    const flight: InFlight = { job, controller, done: Promise.resolve() };
    flight.done = this.process(job, controller).finally(() => {
      this.inflight.delete(job.id);
      this.released.delete(job.id);
      this.wake();
    });
    this.inflight.set(job.id, flight);
  }

  private async process(job: ClaimedJob, controller: AbortController): Promise<void> {
    let leaseLost = false;
    const heartbeat = setInterval(
      () => {
        this.store
          .heartbeat(job, this.leaseMs)
          .then((ok) => {
            if (!ok && !controller.signal.aborted) {
              leaseLost = true;
              controller.abort(new Error('lease lost'));
            }
          })
          .catch((err: unknown) => this.logger.warn(`heartbeat failed: ${String(err)}`));
      },
      Math.max(100, Math.floor(this.leaseMs / 3)),
    );
    const started = performance.now();
    const tracer = trace.getTracer(TRACER_NAME);
    await tracer.startActiveSpan(
      'job_process',
      {
        kind: SpanKind.CONSUMER,
        links: linkFromTraceParent(job.traceParent),
        attributes: { queue: job.queue, job_id: job.id, attempt: job.attempts },
      },
      async (span) => {
        try {
          const payload = parsePayload(this.queue, job.payload);
          const ctx: JobContext<Q> = {
            jobId: job.id,
            queue: this.queue,
            organizationId: job.organizationId,
            payload,
            attempt: job.attempts,
            maxAttempts: job.maxAttempts,
            traceParent: job.traceParent,
            signal: controller.signal,
          };
          await this.handler(ctx);
          if (leaseLost || this.released.has(job.id)) return;
          const completed = await this.store.complete(job);
          span.setAttribute('outcome', completed ? 'succeeded' : 'lease_lost');
        } catch (err) {
          span.recordException(err instanceof Error ? err : new Error(String(err)));
          span.setStatus({ code: SpanStatusCode.ERROR });
          // A released or re-claimed job belongs to someone else now.
          if (leaseLost || this.released.has(job.id)) return;
          const state = await this.store
            .fail(job, classifyFailure(err))
            .catch((failErr: unknown) => {
              this.logger.error(
                `recording the failure of job ${job.id} failed: ${String(failErr)}`,
              );
              return null;
            });
          span.setAttribute('outcome', state ?? 'lease_lost');
        } finally {
          clearInterval(heartbeat);
          recordHistogram('worker_duration_seconds', (performance.now() - started) / 1000, {
            queue: job.queue,
          });
          incCounter('jobs_processed_total', { queue: job.queue });
          span.end();
        }
      },
    );
  }

  private async listen(): Promise<void> {
    if (!this.pool || !this.running) return;
    try {
      const client = await this.pool.connect();
      client.on('notification', () => this.wake());
      client.on('error', () => this.onListenerLost(client));
      // The channel is a fixed queue name (validated), quoted as an identifier.
      await client.query(`LISTEN "jobs_${this.queue}"`);
      this.listener = client;
      // A notification may have been missed while (re)connecting.
      this.wake();
    } catch (err) {
      this.logger.warn(`LISTEN jobs_${this.queue} failed, polling only: ${String(err)}`);
      this.scheduleListen();
    }
  }

  private onListenerLost(client: PoolClient): void {
    if (this.listener !== client) return;
    this.listener = null;
    client.release(true);
    this.scheduleListen();
  }

  private scheduleListen(): void {
    if (!this.running || this.listenerRetry) return;
    this.listenerRetry = setTimeout(() => {
      this.listenerRetry = null;
      void this.listen();
    }, 2_000);
    this.listenerRetry.unref();
  }

  private async closeListener(): Promise<void> {
    const client = this.listener;
    this.listener = null;
    if (!client) return;
    try {
      await client.query(`UNLISTEN "jobs_${this.queue}"`);
      client.release();
    } catch {
      client.release(true);
    }
  }
}

/** ProviderError{rate_limited} re-queues at retry-after; PermanentJobError goes dead; else backoff. */
export function classifyFailure(err: unknown): FailKind {
  if (err instanceof ProviderError && err.kind === 'rate_limited') {
    return { kind: 'rate_limited', retryAfterMs: err.retryAfterMs ?? 60_000 };
  }
  const message = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
  if (err instanceof PermanentJobError) return { kind: 'permanent', error: message };
  if (err instanceof ProviderError && !err.retryable) return { kind: 'permanent', error: message };
  return { kind: 'transient', error: message };
}
