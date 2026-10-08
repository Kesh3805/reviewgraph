import { randomInt } from 'node:crypto';
import request from 'supertest';
import { signServiceToken, ServiceKeyRing } from '../src/internal/service-token';
import { RepositorySyncService } from '../src/repositories/sync.service';
import { PullRequestSyncService } from '../src/reviews/pull-request-sync.service';
import { ReviewOrchestrator } from '../src/reviews/review-orchestrator';
import type { ProviderPullRequest, PullRequestHeadEvent } from '../src/providers/ports';
import { VALID_ENV } from '../test/helpers';
import { type Harness, seedPr, seedRepo, sha, startHarness } from './github-harness';

const RING = ServiceKeyRing.from([
  { kid: 'default', secret: Buffer.from(VALID_ENV.SERVICE_JWT_SECRET as string, 'utf8') },
]);

describe('GitHub sync and clone credentials (integration)', () => {
  let h: Harness;

  beforeAll(async () => {
    h = await startHarness();
  });
  afterAll(async () => {
    await h.close();
  });
  beforeEach(() => {
    h.jobs.reset();
  });

  const prModel = (
    s: Awaited<ReturnType<typeof seedRepo>>,
    number: number,
    head: string,
    updatedAt: string,
    title = 'title',
  ): ProviderPullRequest => ({
    ref: {
      provider: 'github',
      installationId: s.installationId,
      owner: s.owner,
      name: s.name,
      number,
    },
    title,
    state: 'open',
    draft: false,
    baseSha: sha('0'),
    headSha: head,
    baseRef: 'main',
    headRef: 'feature',
    author: { login: 'octo-dev', isBot: false },
    labels: [],
    updatedAt,
  });

  it('pr_upsert_ignores_older_event', async () => {
    const s = await seedRepo(h);
    const sync = h.app.get(PullRequestSyncService);
    const target = { organizationId: s.org.organizationId, repositoryId: s.repositoryId };
    const created = await sync.upsert(
      target,
      prModel(s, 7, sha('b'), '2026-10-02T10:00:00Z', 'new'),
    );
    expect(created).toMatchObject({ applied: true, created: true });
    const stale = await sync.upsert(target, prModel(s, 7, sha('a'), '2026-10-02T09:00:00Z', 'old'));
    expect(stale).toEqual({ pullRequestId: created.pullRequestId, applied: false, created: false });
    const row = await h.admin
      .selectFrom('pull_requests')
      .selectAll()
      .where('id', '=', created.pullRequestId)
      .executeTakeFirstOrThrow();
    expect(row.title).toBe('new');
    expect(row.head_sha).toBe(sha('b'));
    expect(row.provider_updated_at?.toISOString()).toBe('2026-10-02T10:00:00.000Z');
    // A newer event updates metadata but never the head (SUP-001 owns head moves).
    const newer = await sync.upsert(
      target,
      prModel(s, 7, sha('c'), '2026-10-02T11:00:00Z', 'newer'),
    );
    expect(newer.applied).toBe(true);
    const after = await h.admin
      .selectFrom('pull_requests')
      .select(['title', 'head_sha'])
      .where('id', '=', created.pullRequestId)
      .executeTakeFirstOrThrow();
    expect(after).toEqual({ title: 'newer', head_sha: sha('b') });
  });

  it('repo_404_marks_access_lost', async () => {
    const s = await seedRepo(h);
    h.github.repos.delete(s.fullName);
    const result = await h.app
      .get(RepositorySyncService)
      .upsertFromEvent(
        { provider: 'github', installationId: s.installationId, owner: s.owner, name: s.name },
        { refresh: true },
      );
    expect(result).toEqual({ status: 'access_lost', repositoryId: s.repositoryId });
    const row = await h.admin
      .selectFrom('repositories')
      .select(['enabled', 'access_state'])
      .where('id', '=', s.repositoryId)
      .executeTakeFirstOrThrow();
    expect(row).toEqual({ enabled: false, access_state: 'access_lost' });
  });

  it('repository sync reads the authoritative default branch', async () => {
    const s = await seedRepo(h);
    h.github.addRepo(s.fullName, s.providerRepoId, 'trunk');
    const result = await h.app
      .get(RepositorySyncService)
      .upsertFromEvent(
        { provider: 'github', installationId: s.installationId, owner: s.owner, name: s.name },
        { refresh: true },
      );
    expect(result.status).toBe('synced');
    const row = await h.admin
      .selectFrom('repositories')
      .select('default_branch')
      .where('id', '=', s.repositoryId)
      .executeTakeFirstOrThrow();
    expect(row.default_branch).toBe('trunk');
  });

  it('a head event syncs the PR from the fake server and starts one run', async () => {
    const s = await seedRepo(h);
    const number = randomInt(1, 100_000);
    h.github.setPull(s.fullName, { number, headSha: sha('d'), baseSha: sha('0') });
    const event: PullRequestHeadEvent = {
      type: 'pull_request_head',
      kind: 'opened',
      provider: 'github',
      deliveryId: 'd-sync-1',
      installationId: s.installationId,
      repo: { provider: 'github', installationId: s.installationId, owner: s.owner, name: s.name },
      pr: {
        provider: 'github',
        installationId: s.installationId,
        owner: s.owner,
        name: s.name,
        number,
      },
      // A stale payload head: the authoritative read wins.
      headSha: sha('c'),
      baseSha: sha('0'),
      baseRef: 'main',
      author: { login: 'octo-dev', isBot: false },
      draft: false,
    };
    await expect(h.app.get(ReviewOrchestrator).handle(event)).resolves.toBe('started');
    const pr = await h.admin
      .selectFrom('pull_requests')
      .select(['id', 'head_sha', 'base_sha'])
      .where('repository_id', '=', s.repositoryId)
      .where('provider_number', '=', number)
      .executeTakeFirstOrThrow();
    expect(pr.head_sha).toBe(sha('d'));
    const runs = await h.admin
      .selectFrom('review_runs')
      .select(['state', 'head_sha', 'idempotency_key'])
      .where('pull_request_id', '=', pr.id)
      .execute();
    expect(runs).toEqual([
      {
        state: 'RECEIVED',
        head_sha: sha('d'),
        idempotency_key: `pr-review:github:${s.providerRepoId}:${number}:${sha('d')}`,
      },
    ]);
    expect(h.jobs.enqueued).toHaveLength(1);
  });

  describe('clone credential broker (GH-006)', () => {
    const issue = (id: string, token?: string) => {
      const req = request(h.app.getHttpServer()).post(
        `/internal/repositories/${id}/clone-credentials`,
      );
      return token ? req.set('Authorization', `Bearer ${token}`) : req;
    };
    const tokenFor = (
      repo: string,
      scope: ('clone-credentials' | 'graph:read')[] = ['clone-credentials'],
    ) => signServiceToken(RING, { iss: 'rg-worker', aud: 'rg-api', sub: 'worker-1', scope, repo });

    it('requires_service_token', async () => {
      const s = await seedPr(h);
      expect((await issue(s.repositoryId)).status).toBe(401);
      expect(
        (await issue(s.repositoryId, await tokenFor(s.repositoryId, ['graph:read']))).status,
      ).toBe(403);
    });

    it('repo_claim_mismatch_403', async () => {
      const a = await seedPr(h);
      const b = await seedPr(h);
      const res = await issue(a.repositoryId, await tokenFor(b.repositoryId));
      expect(res.status).toBe(403);
      expect(JSON.stringify(res.body)).not.toContain('ghs_');
    });

    it('token_scoped_read_only_single_repo, no_store_header and token_absent_from_logs', async () => {
      const s = await seedPr(h);
      const writes: string[] = [];
      const out = jest.spyOn(process.stdout, 'write').mockImplementation((chunk: unknown) => {
        writes.push(String(chunk));
        return true;
      });
      const err = jest.spyOn(process.stderr, 'write').mockImplementation((chunk: unknown) => {
        writes.push(String(chunk));
        return true;
      });
      let res;
      try {
        res = await issue(s.repositoryId, await tokenFor(s.repositoryId));
      } finally {
        out.mockRestore();
        err.mockRestore();
      }
      expect(res.status).toBe(200);
      expect(res.headers['cache-control']).toBe('no-store');
      expect(res.body).toMatchObject({
        username: 'x-access-token',
        clone_url: `${h.github.url}/${s.fullName}.git`,
      });
      expect(res.body.token).toMatch(/^ghs_fake/);
      expect(res.body.clone_url).not.toContain(res.body.token);
      const mint = h.github.requests.filter(
        (r) => r.path === `/app/installations/${s.installationId}/access_tokens`,
      );
      expect(mint.at(-1)?.body).toEqual({
        repository_ids: [s.providerRepoId],
        permissions: { contents: 'read' },
      });
      expect(writes.join('')).not.toContain(res.body.token);
    });

    it('suspended installation answers 409 installation_suspended', async () => {
      const s = await seedPr(h);
      await h.admin
        .updateTable('provider_installations')
        .set({ state: 'suspended', suspended_at: new Date() })
        .where('id', '=', s.org.installationId)
        .execute();
      const res = await issue(s.repositoryId, await tokenFor(s.repositoryId));
      expect(res.status).toBe(409);
      expect(res.body.code).toBe('installation_suspended');
    });

    it('unknown repository answers 404', async () => {
      const id = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2c';
      expect((await issue(id, await tokenFor(id))).status).toBe(404);
    });
  });
});
