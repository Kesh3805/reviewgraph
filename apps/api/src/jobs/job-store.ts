import { context, isSpanContextValid, trace, TraceFlags, type Link } from '@opentelemetry/api';
import type { JobQueueName } from './payloads';

/** A job as claimed by a worker: the fence for heartbeat, complete and fail is (lockedBy, attempts). */
export interface ClaimedJob {
  id: string;
  queue: JobQueueName;
  organizationId: string;
  payload: unknown;
  attempts: number;
  maxAttempts: number;
  lockedBy: string;
  traceParent: string | null;
  /** Seconds between becoming claimable and the claim. */
  waitSeconds: number;
}

export type FailKind =
  | { kind: 'transient'; error: string }
  | { kind: 'permanent'; error: string }
  | { kind: 'rate_limited'; retryAfterMs: number };

/** The worker-side operations a consumer needs (implemented by PgJobQueue; faked in tests). */
export interface JobStore {
  claim(
    queues: readonly JobQueueName[],
    workerId: string,
    leaseMs: number,
  ): Promise<ClaimedJob | null>;
  /** Extends the lease; false means the lease was lost and the handler must stop. */
  heartbeat(job: ClaimedJob, leaseMs: number): Promise<boolean>;
  /** False when the lease was lost (a stale worker cannot complete a re-claimed job). */
  complete(job: ClaimedJob): Promise<boolean>;
  /** The resulting state, or null when the lease was lost. */
  fail(job: ClaimedJob, failure: FailKind): Promise<'queued' | 'dead' | null>;
  /** Returns this worker's running jobs (or the listed ones) to `queued`, refunding the attempt. */
  release(workerId: string, jobIds?: string[]): Promise<number>;
  /** Samples the `queue_depth` gauge (rate limited by the store). */
  sampleDepth(force?: boolean): Promise<void>;
}

const BACKOFF_BASE_MS = 5_000;
const BACKOFF_CAP_MS = 300_000;

/** `min(300 s, 5 s * 2^(attempts-1))` with full jitter (PIPE-001). */
export function backoffMs(attempts: number, random: () => number = Math.random): number {
  const exp = BACKOFF_BASE_MS * 2 ** Math.max(0, attempts - 1);
  return Math.floor(random() * Math.min(BACKOFF_CAP_MS, exp));
}

/** W3C traceparent of the active span, or null outside a trace. */
export function currentTraceParent(): string | null {
  const ctx = trace.getSpan(context.active())?.spanContext();
  if (!ctx || !isSpanContextValid(ctx)) return null;
  return `00-${ctx.traceId}-${ctx.spanId}-${(ctx.traceFlags & 0xff).toString(16).padStart(2, '0')}`;
}

const TRACEPARENT = /^[0-9a-f]{2}-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/;

/** A span link to the producer's trace (`job_process` is linked, not parented, to it). */
export function linkFromTraceParent(traceParent: string | null): Link[] {
  const match = traceParent ? TRACEPARENT.exec(traceParent) : null;
  if (!match) return [];
  const ctx = {
    traceId: match[1]!,
    spanId: match[2]!,
    traceFlags: parseInt(match[3]!, 16) & TraceFlags.SAMPLED,
    isRemote: true,
  };
  return isSpanContextValid(ctx) ? [{ context: ctx }] : [];
}
