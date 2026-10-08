import { randomInt, randomUUID } from 'node:crypto';
import request from 'supertest';
import { PublisherService } from '../src/publisher/publisher.service';
import { SupersessionService } from '../src/reviews/supersession.service';
import { signWebhookBody } from '../src/webhooks/signature';
import { fixture } from '../test/helpers/fixtures';
import {
  type Harness,
  type SeededPr,
  WEBHOOK_SECRET,
  addedPatch,
  runState,
  seedFindings,
  seedPr,
  seedRepo,
  seedRun,
  sha,
  startHarness,
} from './github-harness';

/**
 * SUP-004, API side: the races between webhooks, supersession and publication, against real
 * Postgres and Redis and the stateful fake GitHub. The engine-side cases (worker killed mid
 * VERIFYING, supersession during a model call) need the worker and are not covered here.
 */
const FILE = 'src/app.ts';
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const ACTIVE = ['RECEIVED', 'INDEXING', 'ANALYZING', 'REVIEWING', 'VERIFYING', 'PUBLISHING'];

describe('concurrency (integration, SUP-004)', () => {
  let h: Harness;
  let sup: SupersessionService;
  let publisher: PublisherService;

  beforeAll(async () => {
    h = await startHarness();
    sup = h.app.get(SupersessionService);
    publisher = h.app.get(PublisherService);
  });
  afterAll(async () => {
    await h.close();
  });
  beforeEach(() => {
    h.jobs.reset();
    h.github.faults = {};
    h.github.reviewPostDelayMs = 0;
  });

  const start = (pr: SeededPr, head: string, at?: Date) =>
    sup.startReview({
      organizationId: pr.org.organizationId,
      pullRequestId: pr.pullRequestId,
      headSha: head,
      baseSha: sha('0'),
      trigger: 'webhook',
      prUpdatedAt: at ?? null,
    });

  const runsOf = (pr: SeededPr) =>
    h.admin
      .selectFrom('review_runs')
      .select(['id', 'head_sha', 'state'])
      .where('pull_request_id', '=', pr.pullRequestId)
      .execute();

  async function withFile(pr: SeededPr, head: string): Promise<void> {
    h.github.setPull(pr.fullName, {
      number: pr.number,
      headSha: head,
      files: [
        { filename: FILE, status: 'modified', additions: 20, deletions: 0, patch: addedPatch(20) },
      ],
    });
  }

  it('two_rapid_updates: only the newest head is published', async () => {
    const pr = await seedPr(h, sha('a'));
    const t = Date.now();
    const b = start(pr, sha('b'), new Date(t));
    await sleep(50);
    const c = start(pr, sha('c'), new Date(t + 1000));
    await Promise.all([b, c]);
    const runs = await runsOf(pr);
    const active = runs.filter((r) => ACTIVE.includes(r.state));
    expect(active.map((r) => r.head_sha)).toEqual([sha('c')]);
    const runB = runs.find((r) => r.head_sha === sha('b'));
    if (runB) expect(runB.state).toBe('SUPERSEDED');

    // The engine finishes both: only C reaches the gate as current.
    await withFile(pr, sha('c'));
    for (const run of runs) {
      await h.admin
        .updateTable('review_runs')
        .set({ state: 'PUBLISHING' })
        .where('id', '=', run.id)
        .where('state', '=', 'RECEIVED')
        .execute();
      await seedFindings(h, pr, run.id, [{ path: FILE, line: 2 }]);
    }
    for (const run of runs) await publisher.publish(run.id);
    const posts = h.github
      .reviewPosts()
      .filter(
        (r) => r.path.endsWith(`/pulls/${pr.number}/reviews`) && r.path.includes(pr.fullName),
      );
    expect(posts.map((p) => (p.body as { commit_id: string }).commit_id)).toEqual([sha('c')]);
  });

  it('duplicate_webhook: 10 parallel deliveries with one id give 1 run and 1 job', async () => {
    const s = await seedRepo(h);
    const number = randomInt(1, 100_000);
    h.github.setPull(s.fullName, { number, headSha: sha('d'), baseSha: sha('0') });
    const payload = fixture('pull_request.opened.json');
    payload.installation.id = Number(s.installationId);
    payload.repository.name = s.name;
    payload.repository.owner.login = s.owner;
    payload.repository.full_name = s.fullName;
    payload.pull_request.number = number;
    payload.pull_request.head.sha = sha('d');
    const raw = Buffer.from(JSON.stringify(payload));
    const deliveryId = `conc-${randomUUID()}`;
    const send = () =>
      request(h.app.getHttpServer())
        .post('/api/v1/webhooks/github')
        .set('content-type', 'application/json')
        .set('x-github-event', 'pull_request')
        .set('x-github-delivery', deliveryId)
        .set('x-hub-signature-256', signWebhookBody(WEBHOOK_SECRET, raw))
        .send(raw.toString());
    const responses = await Promise.all(Array.from({ length: 10 }, send));
    expect(responses.every((r) => r.status === 202)).toBe(true);
    expect(responses.filter((r) => r.body.accepted === true)).toHaveLength(1);

    // Dispatch is detached from the acknowledgement: wait for the run.
    let runs: { head_sha: string }[] = [];
    for (let i = 0; i < 50 && runs.length === 0; i++) {
      await sleep(100);
      runs = await h.admin
        .selectFrom('review_runs')
        .select('head_sha')
        .where('repository_id', '=', s.repositoryId)
        .execute();
    }
    await sleep(300);
    runs = await h.admin
      .selectFrom('review_runs')
      .select('head_sha')
      .where('repository_id', '=', s.repositoryId)
      .execute();
    expect(runs).toEqual([{ head_sha: sha('d') }]);
    expect(h.jobs.enqueued.filter((j) => j.repositoryId === s.repositoryId)).toHaveLength(1);
  });

  it('publish_retry: a dropped POST and concurrent retries still give exactly 1 review', async () => {
    const pr = await seedPr(h, sha('a'));
    await withFile(pr, sha('a'));
    const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
    await seedFindings(h, pr, run, [{ path: FILE, line: 3 }]);
    h.github.faults.reviewPost = 'drop_after_write';
    await expect(publisher.publish(run)).rejects.toBeDefined();
    const outcomes = await Promise.all([publisher.publish(run), publisher.publish(run)]);
    expect(outcomes.sort()).toEqual(expect.arrayContaining(['adopted']));
    expect(h.github.reviews.filter((r) => r.repo === pr.fullName)).toHaveLength(1);
    expect(await runState(h, run)).toBe('COMPLETED');
  });

  it('superseded_during_publish: no review for an obsolete head in 100 interleavings', async () => {
    for (let i = 0; i < 100; i++) {
      const pr = await seedPr(h, sha('a'), randomInt(1, 1_000_000));
      await withFile(pr, sha('a'));
      const run = await seedRun(h, pr, 'PUBLISHING', sha('a'));
      await seedFindings(h, pr, run, [{ path: FILE, line: 4 }]);
      h.github.reviewPostDelayMs = randomInt(0, 15);
      const publishing = (async () => {
        await sleep(randomInt(0, 15));
        return publisher.publish(run);
      })();
      const superseding = (async () => {
        await sleep(randomInt(0, 15));
        return start(pr, sha('b'));
      })();
      const [outcome] = await Promise.all([publishing, superseding]);
      const reviews = h.github.reviews.filter((r) => r.repo === pr.fullName);
      const state = await runState(h, run);
      if (reviews.length > 0) {
        // Posted only while current: the run completed before the head moved.
        expect(outcome).toBe('published');
        expect(state).toBe('COMPLETED');
      } else {
        expect(outcome).toBe('skipped_superseded');
        expect(state).toBe('SUPERSEDED');
      }
      expect(reviews.length).toBeLessThanOrEqual(1);
    }
  }, 180_000);
});
