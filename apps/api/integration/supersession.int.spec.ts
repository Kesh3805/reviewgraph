import { randomInt } from 'node:crypto';
import { sql } from 'kysely';
import { DbService } from '../src/db/db.module';
import { isDbUnavailable } from '../src/db/errors';
import { PublishGate } from '../src/publisher/publish-gate';
import { SupersessionService, type StartReviewInput } from '../src/reviews/supersession.service';
import {
  type Harness,
  type SeededPr,
  holdPrLock,
  runState,
  seedPr,
  seedRun,
  sha,
  startHarness,
} from './github-harness';

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('supersession and the publish gate (integration)', () => {
  let h: Harness;
  let sup: SupersessionService;
  let gate: PublishGate;
  let dbs: DbService;

  beforeAll(async () => {
    h = await startHarness();
    sup = h.app.get(SupersessionService);
    gate = h.app.get(PublishGate);
    dbs = h.app.get(DbService);
  });
  afterAll(async () => {
    await h.close();
  });
  beforeEach(() => {
    h.jobs.reset();
  });

  const start = (pr: SeededPr, head: string, extra: Partial<StartReviewInput> = {}) =>
    sup.startReview({
      organizationId: pr.org.organizationId,
      pullRequestId: pr.pullRequestId,
      headSha: head,
      baseSha: sha('0'),
      trigger: 'webhook',
      ...extra,
    });

  const runsOf = (pr: SeededPr) =>
    h.admin
      .selectFrom('review_runs')
      .selectAll()
      .where('pull_request_id', '=', pr.pullRequestId)
      .orderBy('created_at')
      .execute();

  const prRow = (pr: SeededPr) =>
    h.admin
      .selectFrom('pull_requests')
      .selectAll()
      .where('id', '=', pr.pullRequestId)
      .executeTakeFirstOrThrow();

  describe('SUP-001', () => {
    it('new_head_supersedes_running_and_queued', async () => {
      const pr = await seedPr(h, sha('a'));
      const running = await seedRun(h, pr, 'REVIEWING', sha('a'));
      const result = await start(pr, sha('b'));
      expect(result.outcome).toBe('started');
      if (result.outcome !== 'started') return;
      expect(result.superseded).toEqual([running]);
      expect(h.jobs.cancelled).toEqual([running]);
      const runs = await runsOf(pr);
      const old = runs.find((r) => r.id === running)!;
      expect(old).toMatchObject({
        state: 'SUPERSEDED',
        superseded_by: result.reviewRunId,
        superseded_by_head: sha('b'),
      });
      expect(old.superseded_at).not.toBeNull();
      const fresh = runs.find((r) => r.id === result.reviewRunId)!;
      expect(fresh).toMatchObject({ state: 'RECEIVED', head_sha: sha('b'), trigger: 'webhook' });
      expect((await prRow(pr)).head_sha).toBe(sha('b'));
      expect(h.jobs.enqueued).toEqual([
        {
          reviewRunId: result.reviewRunId,
          organizationId: pr.org.organizationId,
          repositoryId: pr.repositoryId,
          idempotencyKey: `pr-review:github:${pr.providerRepoId}:${pr.number}:${sha('b')}`,
        },
      ]);

      // A queued (RECEIVED) run is superseded the same way by the next head.
      const next = await start(pr, sha('c'));
      expect(next.outcome).toBe('started');
      expect(h.jobs.cancelled).toEqual([running, result.reviewRunId]);
      const active = (await runsOf(pr)).filter((r) =>
        ['RECEIVED', 'INDEXING', 'ANALYZING', 'REVIEWING', 'VERIFYING', 'PUBLISHING'].includes(
          r.state,
        ),
      );
      expect(active.map((r) => r.head_sha)).toEqual([sha('c')]);
    });

    it('same_head_twice_one_run', async () => {
      const pr = await seedPr(h, sha('a'));
      const first = await start(pr, sha('b'));
      const second = await start(pr, sha('b'));
      expect(first.outcome).toBe('started');
      expect(second.outcome).toBe('duplicate');
      expect('reviewRunId' in second && second.reviewRunId).toBe(
        'reviewRunId' in first && first.reviewRunId,
      );
      expect(await runsOf(pr)).toHaveLength(1);
      expect(h.jobs.enqueued).toHaveLength(1);
    });

    it('stale_event_does_not_supersede_newer', async () => {
      const pr = await seedPr(h, sha('a'));
      const newer = await start(pr, sha('c'), { prUpdatedAt: new Date('2026-10-02T12:00:00Z') });
      expect(newer.outcome).toBe('started');
      const late = await start(pr, sha('b'), { prUpdatedAt: new Date('2026-10-02T11:00:00Z') });
      expect(late).toEqual({ outcome: 'stale_event' });
      expect((await prRow(pr)).head_sha).toBe(sha('c'));
      const runs = await runsOf(pr);
      expect(runs.map((r) => [r.head_sha, r.state])).toEqual([[sha('c'), 'RECEIVED']]);
    });

    it('rollback_leaves_no_partial_state', async () => {
      const pr = await seedPr(h, sha('a'));
      const running = await seedRun(h, pr, 'ANALYZING', sha('a'));
      h.jobs.failEnqueue = true;
      await expect(start(pr, sha('b'))).rejects.toThrow('queue down');
      expect((await prRow(pr)).head_sha).toBe(sha('a'));
      const runs = await runsOf(pr);
      expect(runs.map((r) => [r.id, r.state])).toEqual([[running, 'ANALYZING']]);
    });

    it('completed_runs_untouched', async () => {
      const pr = await seedPr(h, sha('a'));
      const done = await seedRun(h, pr, 'COMPLETED', sha('a'));
      const result = await start(pr, sha('b'));
      expect(result.outcome).toBe('started');
      expect(await runState(h, done)).toBe('COMPLETED');
      expect(h.jobs.cancelled).toEqual([]);
    });

    it('a head that comes back after supersession is reviewed again (idempotently)', async () => {
      const pr = await seedPr(h, sha('a'));
      const a = await start(pr, sha('a'));
      const b = await start(pr, sha('b'));
      const again = await start(pr, sha('a'));
      const repeat = await start(pr, sha('a'));
      expect([a.outcome, b.outcome, again.outcome, repeat.outcome]).toEqual([
        'started',
        'started',
        'started',
        'duplicate',
      ]);
      const runs = await runsOf(pr);
      expect(runs.filter((r) => r.state === 'RECEIVED').map((r) => r.head_sha)).toEqual([sha('a')]);
    });

    it('a manual re-review of a completed head starts a retry run', async () => {
      const pr = await seedPr(h, sha('a'));
      const done = await seedRun(h, pr, 'COMPLETED', sha('a'));
      const manual = await start(pr, sha('a'), {
        trigger: 'manual',
        depth: 'full',
        triggerSuffix: 'manual:1',
      });
      expect(manual.outcome).toBe('started');
      const runs = await runsOf(pr);
      const retry = runs.find((r) => r.id !== done)!;
      expect(retry).toMatchObject({ retry_of: done, depth: 'full', trigger: 'manual' });
    });

    it('a closed PR starts nothing', async () => {
      const pr = await seedPr(h, sha('a'));
      await h.admin
        .updateTable('pull_requests')
        .set({ state: 'closed' })
        .where('id', '=', pr.pullRequestId)
        .execute();
      await expect(start(pr, sha('b'))).resolves.toEqual({ outcome: 'pr_not_open' });
    });

    it('cancelActiveRuns cancels and records the closed state', async () => {
      const pr = await seedPr(h, sha('a'));
      const run = await seedRun(h, pr, 'REVIEWING', sha('a'));
      const cancelled = await sup.cancelActiveRuns(pr.org.organizationId, pr.pullRequestId, {
        closedState: 'merged',
      });
      expect(cancelled).toEqual([run]);
      expect(await runState(h, run)).toBe('CANCELLED');
      expect((await prRow(pr)).state).toBe('merged');
      expect(h.jobs.cancelled).toEqual([run]);
    });
  });

  describe('SUP-003 publish gate', () => {
    const acquire = (pr: SeededPr, runId: string) =>
      dbs.withTx(pr.org.organizationId, (trx) => gate.acquire(trx, runId));

    it('proceeds for a current PUBLISHING run', async () => {
      const pr = await seedPr(h, sha('a'));
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await expect(acquire(pr, run)).resolves.toMatchObject({ proceed: true });
    });

    it('gate_skips_when_head_moved', async () => {
      const pr = await seedPr(h, sha('a'));
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await h.admin
        .updateTable('pull_requests')
        .set({ head_sha: sha('b') })
        .where('id', '=', pr.pullRequestId)
        .execute();
      await expect(acquire(pr, run)).resolves.toMatchObject({
        proceed: false,
        reason: 'head_moved',
      });
    });

    it('gate_skips_when_superseded', async () => {
      const pr = await seedPr(h, sha('a'));
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await start(pr, sha('b'));
      await expect(acquire(pr, run)).resolves.toMatchObject({
        proceed: false,
        reason: 'superseded',
      });
    });

    it('gate_skips_when_pr_closed', async () => {
      const pr = await seedPr(h, sha('a'));
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await h.admin
        .updateTable('pull_requests')
        .set({ state: 'closed' })
        .where('id', '=', pr.pullRequestId)
        .execute();
      await expect(acquire(pr, run)).resolves.toMatchObject({
        proceed: false,
        reason: 'pr_closed',
      });
    });

    it('gate skips when the repository was disabled', async () => {
      const pr = await seedPr(h, sha('a'));
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await h.admin
        .updateTable('repositories')
        .set({ enabled: false, access_state: 'removed' })
        .where('id', '=', pr.repositoryId)
        .execute();
      await expect(acquire(pr, run)).resolves.toMatchObject({
        proceed: false,
        reason: 'repository_disabled',
      });
    });

    it('supersede_waits_for_inflight_publish_then_applies', async () => {
      const pr = await seedPr(h, sha('a'));
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      let releasePublish!: () => void;
      const publishing = new Promise<void>((r) => (releasePublish = r));
      let gateHeld!: () => void;
      const held = new Promise<void>((r) => (gateHeld = r));
      const publish = dbs.withTx(pr.org.organizationId, async (trx) => {
        const g = await gate.acquire(trx, run);
        expect(g.proceed).toBe(true);
        gateHeld();
        await publishing; // the review POST
        await trx
          .updateTable('review_runs')
          .set({ state: 'COMPLETED', completed_at: new Date() })
          .where('id', '=', run)
          .execute();
      });
      await held;
      let settled = false;
      const superseding = start(pr, sha('b')).then((r) => {
        settled = true;
        return r;
      });
      await sleep(400);
      expect(settled).toBe(false);
      releasePublish();
      await publish;
      const result = await superseding;
      expect(result.outcome).toBe('started');
      // The published run completed; supersession found nothing active to supersede.
      expect(await runState(h, run)).toBe('COMPLETED');
      expect('superseded' in result && result.superseded).toEqual([]);
    });

    it('webhook_lock_timeout_returns_503', async () => {
      const pr = await seedPr(h, sha('a'));
      const lock = await holdPrLock(h, pr.pullRequestId);
      try {
        const err = await start(pr, sha('b')).then(
          () => null,
          (e: unknown) => e,
        );
        expect((err as { code?: string }).code).toBe('55P03');
        // The problem filter maps it to 503 + Retry-After, so GitHub (or a caller) retries.
        expect(isDbUnavailable(err)).toBe(true);
      } finally {
        await lock.release();
      }
      expect((await prRow(pr)).head_sha).toBe(sha('a'));
    }, 15_000);

    it('lock_order_no_deadlock (100 interleavings)', async () => {
      for (let i = 0; i < 100; i++) {
        const pr = await seedPr(h, sha('a'), randomInt(1, 1_000_000));
        const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
        let posted = false;
        const publish = (async () => {
          await sleep(randomInt(0, 5));
          return dbs.withTx(pr.org.organizationId, async (trx) => {
            const g = await gate.acquire(trx, run);
            if (!g.proceed) return;
            await sleep(randomInt(0, 5));
            posted = true;
            await trx
              .updateTable('review_runs')
              .set({ state: 'COMPLETED', completed_at: sql<Date>`now()` })
              .where('id', '=', run)
              .execute();
          });
        })();
        const supersede = (async () => {
          await sleep(randomInt(0, 5));
          return start(pr, sha('b'));
        })();
        await Promise.all([publish, supersede]);
        const state = await runState(h, run);
        // Posted means the run completed before the head moved; otherwise it was superseded.
        expect(state).toBe(posted ? 'COMPLETED' : 'SUPERSEDED');
      }
    }, 120_000);
  });
});
