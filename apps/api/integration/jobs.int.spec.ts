import { randomUUID } from 'node:crypto';
import { Client, type Notification } from 'pg';
import { sql } from 'kysely';
import { ProviderError } from '../src/providers/ports';
import type { AppConfig } from '../src/config/config.module';
import { DbService } from '../src/db/db.module';
import { createKysely, createPool } from '../src/db/kysely.provider';
import { PermanentJobError } from '../src/jobs/job-queue';
import { PgJobQueue } from '../src/jobs/pg-job-queue';
import { adminDb, cleanup, seedOrg, type SeededOrg } from './seed';

const LEASE_MS = 3_000;

/** Waits until `check` holds (polling), or fails after `ms`. */
async function eventually(check: () => Promise<boolean>, ms = 10_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error('condition not met in time');
}

describe('PgJobQueue (integration)', () => {
  const admin = adminDb();
  const pool = createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max: 6 });
  const db = createKysely(pool);
  const dbs = new DbService(db, { DB_APP_ROLE: 'rg_api' } as AppConfig);
  const queue = new PgJobQueue(dbs, pool);
  let org: SeededOrg;

  beforeAll(async () => {
    org = await seedOrg(admin, 1);
  });

  // The queue table is shared: start every test from an empty queue so claims are deterministic.
  beforeEach(async () => {
    await admin.deleteFrom('jobs').execute();
  });

  afterAll(async () => {
    await queue.beforeApplicationShutdown();
    await cleanup(admin, [org.organizationId]);
    await admin.destroy();
    await db.destroy();
  });

  const publish = (key = `publish:${randomUUID()}`, maxAttempts?: number) =>
    dbs.withTx(org.organizationId, (trx) =>
      queue.enqueue(trx, {
        queue: 'review-publish',
        idempotencyKey: key,
        organizationId: org.organizationId,
        payload: { review_run_id: randomUUID() },
        maxAttempts,
      }),
    );

  const jobRow = (id: string) =>
    admin.selectFrom('jobs').selectAll().where('id', '=', id).executeTakeFirstOrThrow();

  it('enqueue_rolls_back_with_transaction', async () => {
    const key = `publish:${randomUUID()}`;
    await expect(
      dbs.withTx(org.organizationId, async (trx) => {
        await queue.enqueue(trx, {
          queue: 'review-publish',
          idempotencyKey: key,
          organizationId: org.organizationId,
          payload: { review_run_id: randomUUID() },
        });
        throw new Error('state change failed');
      }),
    ).rejects.toThrow('state change failed');
    const rows = await admin.selectFrom('jobs').where('idempotency_key', '=', key).execute();
    expect(rows).toEqual([]);
  });

  it('duplicate_idempotency_key_returns_created_false', async () => {
    const key = `publish:${randomUUID()}`;
    const first = await publish(key);
    const second = await publish(key);
    expect(first.created).toBe(true);
    expect(second).toEqual({ jobId: first.jobId, created: false });
    const rows = await admin.selectFrom('jobs').where('idempotency_key', '=', key).execute();
    expect(rows).toHaveLength(1);
  });

  it('rejects payloads that are not ids', async () => {
    await expect(
      dbs.withTx(org.organizationId, (trx) =>
        queue.enqueue(trx, {
          queue: 'review-publish',
          idempotencyKey: `publish:${randomUUID()}`,
          organizationId: org.organizationId,
          payload: { review_run_id: 'not a uuid' },
        }),
      ),
    ).rejects.toThrow();
  });

  it('notify_delivered_after_commit_only', async () => {
    const listener = new Client({ connectionString: process.env.RG_TEST_DATABASE_URL! });
    await listener.connect();
    const received: Notification[] = [];
    listener.on('notification', (n) => received.push(n));
    await listener.query('LISTEN "jobs_review-publish"');
    try {
      let enqueuedId = '';
      await dbs.withTx(org.organizationId, async (trx) => {
        const result = await queue.enqueue(trx, {
          queue: 'review-publish',
          idempotencyKey: `publish:${randomUUID()}`,
          organizationId: org.organizationId,
          payload: { review_run_id: randomUUID() },
        });
        enqueuedId = result.jobId;
        // Give the server time to deliver anything it would deliver before the commit.
        await new Promise((r) => setTimeout(r, 300));
        expect(received).toEqual([]);
      });
      await eventually(() => Promise.resolve(received.length > 0), 3_000);
      expect(received[0]).toMatchObject({ channel: 'jobs_review-publish', payload: enqueuedId });

      // A duplicate enqueue does not notify.
      received.length = 0;
      const row = await jobRow(enqueuedId);
      await publish(row.idempotency_key);
      await new Promise((r) => setTimeout(r, 300));
      expect(received).toEqual([]);
    } finally {
      await listener.end();
    }
  });

  it('heartbeat_extends_lease', async () => {
    const { jobId } = await publish();
    const job = (await queue.claim(['review-publish'], 'worker-a', LEASE_MS))!;
    expect(job.id).toBe(jobId);
    const before = (await jobRow(jobId)).locked_until!;
    await new Promise((r) => setTimeout(r, 50));
    expect(await queue.heartbeat(job, 60_000)).toBe(true);
    const after = (await jobRow(jobId)).locked_until!;
    expect(after.getTime()).toBeGreaterThan(before.getTime() + 50_000);
    // Only the owner of the current attempt may extend it.
    expect(await queue.heartbeat({ ...job, lockedBy: 'worker-b' }, 60_000)).toBe(false);
    expect(await queue.heartbeat({ ...job, attempts: job.attempts + 1 }, 60_000)).toBe(false);
  });

  it('shutdown_releases_unfinished_job', async () => {
    const { jobId } = await publish();
    let started = false;
    const handle = queue.consume(
      'review-publish',
      async (ctx) => {
        started = true;
        // Never finishes on its own; it only reacts to the abort signal.
        await new Promise<void>((resolve) => ctx.signal.addEventListener('abort', () => resolve()));
      },
      { leaseMs: LEASE_MS, pollMs: 100 },
    );
    await eventually(() => Promise.resolve(started));
    expect((await jobRow(jobId)).state).toBe('running');
    await handle.stop(200);
    const row = await jobRow(jobId);
    expect(row).toMatchObject({ state: 'queued', locked_by: null, locked_until: null });
    // The attempt is refunded: a shutdown is not the job's fault.
    expect(row.attempts).toBe(0);
  });

  it('consumes, completes and wakes on notify', async () => {
    const seen: string[] = [];
    const handle = queue.consume(
      'review-publish',
      (ctx) => {
        seen.push(ctx.jobId);
        return Promise.resolve();
      },
      // A long poll interval: only the notification can wake the consumer quickly.
      { leaseMs: LEASE_MS, pollMs: 30_000 },
    );
    try {
      await new Promise((r) => setTimeout(r, 300));
      const { jobId } = await publish();
      await eventually(async () => (await jobRow(jobId)).state === 'succeeded', 5_000);
      expect(seen).toEqual([jobId]);
    } finally {
      await handle.stop(1_000);
    }
  });

  it('rate_limited_requeues_with_retry_after', async () => {
    const { jobId } = await publish();
    const handle = queue.consume(
      'review-publish',
      () =>
        Promise.reject(
          new ProviderError('rate_limited', 'secondary rate limit', { retryAfterMs: 120_000 }),
        ),
      { leaseMs: LEASE_MS, pollMs: 100 },
    );
    try {
      await eventually(async () => (await jobRow(jobId)).rate_limit_requeues === 1);
    } finally {
      await handle.stop(1_000);
    }
    const row = await jobRow(jobId);
    expect(row.state).toBe('queued');
    // The claim's attempt was refunded.
    expect(row.attempts).toBe(0);
    const delayMs = row.run_after.getTime() - Date.now();
    expect(delayMs).toBeGreaterThan(100_000);
    expect(delayMs).toBeLessThanOrEqual(120_000);
  });

  it('dead_after_max_attempts', async () => {
    const { jobId } = await publish(undefined, 2);
    for (let attempt = 1; attempt <= 2; attempt++) {
      // Make it claimable now regardless of the backoff.
      await admin
        .updateTable('jobs')
        .set({ run_after: sql<Date>`now()` })
        .where('id', '=', jobId)
        .execute();
      const job = (await queue.claim(['review-publish'], 'worker-a', LEASE_MS))!;
      expect(job.attempts).toBe(attempt);
      const state = await queue.fail(job, { kind: 'transient', error: 'boom' });
      expect(state).toBe(attempt < 2 ? 'queued' : 'dead');
    }
    const row = await jobRow(jobId);
    expect(row).toMatchObject({ state: 'dead', attempts: 2, locked_by: null, last_error: 'boom' });
    // A dead job is never claimed again.
    await admin
      .updateTable('jobs')
      .set({ run_after: sql<Date>`now()` })
      .where('id', '=', jobId)
      .execute();
    expect(await queue.claim(['review-publish'], 'worker-a', LEASE_MS)).toBeNull();
  });

  it('a permanent failure goes dead at once', async () => {
    const { jobId } = await publish();
    const handle = queue.consume(
      'review-publish',
      () => Promise.reject(new PermanentJobError('review run is gone')),
      { leaseMs: LEASE_MS, pollMs: 100 },
    );
    try {
      await eventually(async () => (await jobRow(jobId)).state === 'dead');
    } finally {
      await handle.stop(1_000);
    }
    expect((await jobRow(jobId)).attempts).toBe(1);
  });

  it('ts_and_rust_claim_interoperate', async () => {
    // TS enqueues; the claim below is the target-architecture section 5 statement exactly as the
    // Rust worker (pipeline::jobs, PIPE-001) issues it, on a plain connection.
    const { jobId } = await publish();
    const rust = new Client({ connectionString: process.env.RG_TEST_DATABASE_URL! });
    await rust.connect();
    try {
      const claimed = await rust.query<{ id: string; payload: { review_run_id: string } }>(
        `UPDATE jobs SET state='running', locked_by=$1, locked_until=now()+$2::interval,
                attempts=attempts+1
         WHERE id = (SELECT id FROM jobs WHERE queue=ANY($3) AND state='queued'
                       AND run_after<=now()
                     ORDER BY priority DESC, created_at FOR UPDATE SKIP LOCKED LIMIT 1)
         RETURNING *`,
        ['rust-worker-1', '30 seconds', ['review-publish']],
      );
      expect(claimed.rows.map((r) => r.id)).toEqual([jobId]);
      expect(Object.keys(claimed.rows[0]!.payload)).toEqual(['review_run_id']);
      // The TS side sees the claim and cannot claim the same job.
      expect(await queue.claim(['review-publish'], 'ts-worker', LEASE_MS)).toBeNull();
      // And a TS worker cannot complete a job it does not hold (fence on locked_by).
      const row = await jobRow(jobId);
      expect(
        await queue.complete({
          id: jobId,
          queue: 'review-publish',
          organizationId: org.organizationId,
          payload: row.payload,
          attempts: row.attempts,
          maxAttempts: row.max_attempts,
          lockedBy: 'ts-worker',
          traceParent: null,
          waitSeconds: 0,
        }),
      ).toBe(false);
    } finally {
      await rust.end();
    }
  });

  it('cancel_where only cancels queued jobs', async () => {
    const runA = randomUUID();
    const enqueue = (runId: string) =>
      dbs.withTx(org.organizationId, (trx) =>
        queue.enqueue(trx, {
          queue: 'pr-review',
          idempotencyKey: `pr-review:test:${runId}`,
          organizationId: org.organizationId,
          payload: { review_run_id: runId },
        }),
      );
    const a = await enqueue(runA);
    const b = await enqueue(randomUUID());
    const running = await enqueue(runA.replace(/.$/, runA.endsWith('0') ? '1' : '0'));
    await admin
      .updateTable('jobs')
      .set({ state: 'running', locked_by: 'w', locked_until: sql<Date>`now()` })
      .where('id', '=', running.jobId)
      .execute();
    const cancelled = await dbs.withTx(org.organizationId, (trx) =>
      queue.cancelWhere(trx, { reviewRunIds: [runA], queue: 'pr-review' }),
    );
    expect(cancelled).toBe(1);
    expect((await jobRow(a.jobId)).state).toBe('cancelled');
    expect((await jobRow(b.jobId)).state).toBe('queued');
    expect((await jobRow(running.jobId)).state).toBe('running');
  });
});
