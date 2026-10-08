import {
  GithubReconciler,
  RECONCILER_LOCK_KEY,
  etagKey,
  pollDeliveryId,
} from '../src/providers/github/reconciler.service';
import { ReviewOrchestrator } from '../src/reviews/review-orchestrator';
import type { PullRequestHeadEvent } from '../src/providers/ports';
import { type Harness, seedRepo, sha, startHarness } from './github-harness';

describe('polling reconciler (integration)', () => {
  let h: Harness;
  let reconciler: GithubReconciler;

  beforeAll(async () => {
    h = await startHarness();
    reconciler = h.app.get(GithubReconciler);
  });
  afterAll(async () => {
    await h.close();
  });
  beforeEach(async () => {
    h.jobs.reset();
    await h.redis.del(RECONCILER_LOCK_KEY);
  });

  const runsOf = async (repositoryId: string) =>
    h.admin
      .selectFrom('review_runs')
      .select(['head_sha', 'state', 'trigger'])
      .where('repository_id', '=', repositoryId)
      .execute();

  it('missed_webhook_head_enqueued', async () => {
    const s = await seedRepo(h);
    h.github.setPull(s.fullName, { number: 11, headSha: sha('b'), baseSha: sha('0') });
    const report = await reconciler.runCycle();
    expect(report.ran).toBe(true);
    expect(report.synthesized).toBeGreaterThanOrEqual(1);
    expect(await runsOf(s.repositoryId)).toEqual([
      { head_sha: sha('b'), state: 'RECEIVED', trigger: 'reconciler' },
    ]);
    const delivery = await h.admin
      .selectFrom('webhook_deliveries')
      .select(['event', 'status', 'organization_id'])
      .where('delivery_id', '=', pollDeliveryId(s.repositoryId, 11, sha('b')))
      .executeTakeFirstOrThrow();
    expect(delivery).toEqual({
      event: 'poll',
      status: 'processed',
      organization_id: s.org.organizationId,
    });
  });

  it('known_head_not_enqueued and etag_304_no_work', async () => {
    const s = await seedRepo(h);
    h.github.setPull(s.fullName, { number: 12, headSha: sha('c'), baseSha: sha('0') });
    await reconciler.runCycle();
    expect(await runsOf(s.repositoryId)).toHaveLength(1);
    const enqueued = h.jobs.enqueued.length;

    // Same open PRs: the ETag answers 304 and nothing is listed.
    const second = await reconciler.runCycle();
    expect(second.notModified).toBeGreaterThanOrEqual(1);
    expect(h.jobs.enqueued.length).toBe(enqueued);

    // Without the ETag the list is read again, but the head is known.
    await h.redis.del(etagKey(s.repositoryId));
    await reconciler.runCycle();
    expect(await runsOf(s.repositoryId)).toHaveLength(1);
    expect(h.jobs.enqueued.length).toBe(enqueued);

    // A new head is picked up on the next cycle.
    h.github.setPull(s.fullName, { number: 12, headSha: sha('d') });
    await reconciler.runCycle();
    const runs = await runsOf(s.repositoryId);
    expect(runs.find((r) => r.state === 'RECEIVED')?.head_sha).toBe(sha('d'));
    expect(runs.find((r) => r.head_sha === sha('c'))?.state).toBe('SUPERSEDED');
  });

  it('lock_prevents_parallel_cycles', async () => {
    await seedRepo(h);
    const [a, b] = await Promise.all([reconciler.runCycle(), reconciler.runCycle()]);
    expect([a.ran, b.ran].filter(Boolean)).toHaveLength(1);
    await h.redis.set(RECONCILER_LOCK_KEY, 'someone-else', 'EX', 30);
    expect((await reconciler.runCycle()).ran).toBe(false);
  });

  it('race_with_webhook_single_run', async () => {
    const s = await seedRepo(h);
    h.github.setPull(s.fullName, { number: 13, headSha: sha('e'), baseSha: sha('0') });
    const webhook: PullRequestHeadEvent = {
      type: 'pull_request_head',
      kind: 'synchronize',
      provider: 'github',
      deliveryId: 'wh-race-1',
      installationId: s.installationId,
      repo: { provider: 'github', installationId: s.installationId, owner: s.owner, name: s.name },
      pr: {
        provider: 'github',
        installationId: s.installationId,
        owner: s.owner,
        name: s.name,
        number: 13,
      },
      headSha: sha('e'),
      baseSha: sha('0'),
      baseRef: 'main',
      author: { login: 'octo-dev', isBot: false },
      draft: false,
    };
    await Promise.all([reconciler.runCycle(), h.app.get(ReviewOrchestrator).handle(webhook)]);
    const runs = await runsOf(s.repositoryId);
    expect(runs.filter((r) => r.head_sha === sha('e'))).toHaveLength(1);
  });

  it('drafts and bot PRs are not synthesized (same guards as webhooks)', async () => {
    const s = await seedRepo(h);
    h.github.setPull(s.fullName, { number: 14, headSha: sha('f'), draft: true });
    h.github.setPull(s.fullName, {
      number: 15,
      headSha: sha('f'),
      user: { login: 'dependabot[bot]', type: 'Bot' },
    });
    await reconciler.runCycle();
    expect(await runsOf(s.repositoryId)).toEqual([]);
  });
});
