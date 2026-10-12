import { z } from 'zod';
import { ProviderError } from '../../src/providers/ports';
import { classifyFailure, JobConsumer } from '../../src/jobs/consumer';
import { PermanentJobError } from '../../src/jobs/job-queue';
import {
  backoffMs,
  linkFromTraceParent,
  type ClaimedJob,
  type FailKind,
  type JobStore,
} from '../../src/jobs/job-store';
import { JOB_PAYLOAD_SCHEMAS, JOB_QUEUES } from '../../src/jobs/payloads';

/** An in-memory store with one job, recording what the consumer does with it. */
class FakeStore implements JobStore {
  claimed = 0;
  completed: string[] = [];
  failed: FailKind[] = [];
  released: string[][] = [];
  heartbeatOk = true;

  constructor(private readonly jobs: ClaimedJob[]) {}

  claim(): Promise<ClaimedJob | null> {
    const job = this.jobs.shift() ?? null;
    if (job) this.claimed++;
    return Promise.resolve(job);
  }
  heartbeat(): Promise<boolean> {
    return Promise.resolve(this.heartbeatOk);
  }
  complete(job: ClaimedJob): Promise<boolean> {
    this.completed.push(job.id);
    return Promise.resolve(true);
  }
  fail(_job: ClaimedJob, failure: FailKind): Promise<'queued' | 'dead' | null> {
    this.failed.push(failure);
    return Promise.resolve('queued');
  }
  release(_worker: string, ids?: string[]): Promise<number> {
    this.released.push(ids ?? []);
    return Promise.resolve(ids?.length ?? 0);
  }
  sampleDepth(): Promise<void> {
    return Promise.resolve();
  }
}

const job = (id: string): ClaimedJob => ({
  id,
  queue: 'review-publish',
  organizationId: '0190f3a2-0000-7000-8000-000000000001',
  payload: { review_run_id: '0190f3a2-0000-7000-8000-0000000000aa' },
  attempts: 1,
  maxAttempts: 5,
  lockedBy: 'w',
  traceParent: null,
  waitSeconds: 0,
});

async function until(check: () => boolean, ms = 3_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (!check()) {
    if (Date.now() > deadline) throw new Error('condition not met in time');
    await new Promise((r) => setTimeout(r, 10));
  }
}

describe('job payloads', () => {
  it('payload_schemas_have_no_free_text_fields', () => {
    for (const queue of JOB_QUEUES) {
      const schema = z.toJSONSchema(JOB_PAYLOAD_SCHEMAS[queue]) as {
        properties: Record<string, { type?: string; format?: string; pattern?: string }>;
        additionalProperties?: boolean;
      };
      expect(schema.additionalProperties).toBe(false);
      for (const [field, prop] of Object.entries(schema.properties)) {
        if (prop.type === 'string') {
          // Every string is an id or a commit sha, never prose, source or a secret.
          expect([field, Boolean(prop.format === 'uuid' || prop.pattern)]).toEqual([field, true]);
        } else {
          expect([field, prop.type]).toEqual([field, 'boolean']);
        }
      }
    }
  });
});

describe('backoff', () => {
  it('is bounded by min(300 s, 5 s * 2^(attempts-1)) with full jitter', () => {
    expect(backoffMs(1, () => 0.999999)).toBeLessThan(5_000);
    expect(backoffMs(3, () => 0.999999)).toBeLessThan(20_000);
    expect(backoffMs(3, () => 0.999999)).toBeGreaterThan(19_000);
    expect(backoffMs(20, () => 0.999999)).toBeLessThan(300_000);
    expect(backoffMs(20, () => 0)).toBe(0);
  });
});

describe('failure classification', () => {
  it('rate limits carry the retry-after; permanent errors go dead; the rest back off', () => {
    expect(
      classifyFailure(new ProviderError('rate_limited', 'slow down', { retryAfterMs: 42_000 })),
    ).toEqual({ kind: 'rate_limited', retryAfterMs: 42_000 });
    expect(classifyFailure(new PermanentJobError('gone')).kind).toBe('permanent');
    expect(classifyFailure(new Error('flaky')).kind).toBe('transient');
    expect(classifyFailure(new ProviderError('transient', 'timeout')).kind).toBe('transient');
  });

  it('trace parents become span links', () => {
    const tp = '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01';
    expect(linkFromTraceParent(tp)[0]!.context.traceId).toBe('0af7651916cd43dd8448eb211c80319c');
    expect(linkFromTraceParent('garbage')).toEqual([]);
    expect(linkFromTraceParent(null)).toEqual([]);
  });
});

describe('JobConsumer', () => {
  it('completes a successful job and fails a throwing one', async () => {
    const store = new FakeStore([job('a'), job('b')]);
    const consumer = new JobConsumer(
      store,
      'review-publish',
      (ctx) => (ctx.jobId === 'b' ? Promise.reject(new Error('boom')) : Promise.resolve()),
      { pollMs: 20 },
    );
    consumer.start();
    await until(() => store.completed.length === 1 && store.failed.length === 1);
    await consumer.stop(100);
    expect(store.completed).toEqual(['a']);
    expect(store.failed[0]).toMatchObject({ kind: 'transient' });
  });

  it('a lost lease aborts the handler and the job is neither completed nor failed', async () => {
    const store = new FakeStore([job('a')]);
    store.heartbeatOk = false;
    let aborted = false;
    const consumer = new JobConsumer(
      store,
      'review-publish',
      (ctx) =>
        new Promise<void>((resolve) => {
          ctx.signal.addEventListener('abort', () => {
            aborted = true;
            resolve();
          });
        }),
      { pollMs: 20, leaseMs: 300 },
    );
    consumer.start();
    await until(() => aborted);
    await consumer.stop(100);
    expect(store.completed).toEqual([]);
    expect(store.failed).toEqual([]);
  });

  it('shutdown waits for handlers, then releases the unfinished ones', async () => {
    const store = new FakeStore([job('slow')]);
    let started = false;
    const consumer = new JobConsumer(
      store,
      'review-publish',
      (ctx) =>
        new Promise<void>((resolve) => {
          started = true;
          ctx.signal.addEventListener('abort', () => resolve());
        }),
      { pollMs: 20 },
    );
    consumer.start();
    await until(() => started);
    await consumer.stop(50);
    expect(store.released).toEqual([['slow']]);
    expect(store.completed).toEqual([]);
    expect(store.failed).toEqual([]);
  });

  it('never runs more handlers than its concurrency', async () => {
    const store = new FakeStore([job('a'), job('b'), job('c')]);
    let running = 0;
    let peak = 0;
    const consumer = new JobConsumer(
      store,
      'review-publish',
      async () => {
        running++;
        peak = Math.max(peak, running);
        await new Promise((r) => setTimeout(r, 50));
        running--;
      },
      { pollMs: 10, concurrency: 2 },
    );
    consumer.start();
    await until(() => store.completed.length === 3);
    await consumer.stop(100);
    expect(peak).toBe(2);
  });
});
