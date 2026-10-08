import RedisMock from 'ioredis-mock';
import { counterTotal, resetCounterTotals } from '../../../src/common/metrics';
import { GithubAppAuth } from '../../../src/providers/github/app-auth.service';
import type { GithubEventNormalizer } from '../../../src/providers/github/event-normalizer.service';
import type { ActorPermissionLookup } from '../../../src/providers/github/permissions';
import {
  FILES_PER_PAGE,
  GithubRepositoryProvider,
} from '../../../src/providers/github/repository-provider';
import { InstallationTokenCache } from '../../../src/providers/github/token-cache';
import { ProviderError, collectChangedFiles, type PrRef } from '../../../src/providers/ports';
import { testConfig } from '../../helpers';
import { FakeGithub, generateAppKey } from '../../helpers/fake-github';
import { fixture } from '../../helpers/fixtures';

const KEY = generateAppKey();
const PR: PrRef = {
  provider: 'github',
  installationId: '77',
  owner: 'acme',
  name: 'billing',
  number: 42,
};

function file(i: number): Record<string, unknown> {
  return {
    filename: `src/f${i}.ts`,
    status: 'modified',
    additions: 1,
    deletions: 0,
    patch: '@@ -1 +1 @@\n+x',
  };
}

describe('GithubRepositoryProvider (GH-005, GH-006)', () => {
  let github: FakeGithub;
  let provider: GithubRepositoryProvider;

  beforeAll(async () => {
    github = new FakeGithub(KEY.publicKey);
    await github.start();
  });
  afterAll(async () => {
    await github.stop();
  });
  beforeEach(() => {
    github.routes.clear();
    github.patterns.length = 0;
    github.requests.length = 0;
    resetCounterTotals();
    const auth = new GithubAppAuth({
      appId: '1',
      privateKey: KEY.privateKey,
      apiUrl: github.url,
      cache: new InstallationTokenCache(new RedisMock(), Buffer.alloc(32, 3)),
      retries: false,
    });
    provider = new GithubRepositoryProvider(
      testConfig({ GITHUB_API_URL: github.url }),
      auth,
      {} as GithubEventNormalizer,
      {} as ActorPermissionLookup,
    );
  });

  it('fake_server_contract: recorded responses map onto the provider-neutral model', async () => {
    github.route('GET', '/repos/acme/billing/pulls/42', (_req, res) =>
      FakeGithub.json(res, 200, fixture('api/pulls.get.json')),
    );
    github.route('GET', '/repos/acme/billing/pulls/42/files', (_req, res) =>
      FakeGithub.json(res, 200, fixture('api/pulls.files.json')),
    );
    github.route('GET', '/repos/acme/billing', (_req, res) =>
      FakeGithub.json(res, 200, fixture('api/repos.get.json')),
    );

    const pr = await provider.getPullRequest(PR);
    expect(pr).toEqual({
      ref: PR,
      title: 'Charge the invoice total once',
      state: 'open',
      draft: false,
      baseSha: 'aa218f56b14c9653891f9e74264a383fa43fefbd',
      headSha: '6dcb09b5b57875f334f61aebed695e2e4193db5e',
      baseRef: 'main',
      headRef: 'fix-double-charge',
      author: { login: 'octo-dev', isBot: false },
      labels: ['billing', 'bug'],
      updatedAt: '2026-10-02T10:30:00Z',
    });
    // The body is never part of the model (it may hold secrets).
    expect(JSON.stringify(pr)).not.toContain('Internal notes');

    const { files, truncated } = await collectChangedFiles(provider.listChangedFiles(PR));
    expect(truncated).toBe(false);
    expect(files).toHaveLength(2);
    expect(files[1]).toEqual({
      path: 'src/billing/charge.ts',
      previousPath: 'src/billing/payment.ts',
      status: 'renamed',
      additions: 10,
      deletions: 2,
    });

    const repo = await provider.getRepository(PR);
    expect(repo).toEqual({
      ref: PR,
      providerRepoId: '700001',
      defaultBranch: 'trunk',
      isPrivate: true,
      archived: false,
    });
    expect(
      counterTotal('github_api_calls_total', {
        route: 'GET /repos/{owner}/{repo}/pulls/{pull_number}',
        status: 200,
      }),
    ).toBe(1);
  });

  it('list_files_paginates', async () => {
    const all = Array.from({ length: FILES_PER_PAGE * 2 + 5 }, (_, i) => file(i));
    github.route('GET', '/repos/acme/billing/pulls/42/files', (req, res) => {
      const page = Number(req.query.get('page') ?? '1');
      const per = Number(req.query.get('per_page') ?? '30');
      FakeGithub.json(res, 200, all.slice((page - 1) * per, page * per));
    });
    const { files } = await collectChangedFiles(provider.listChangedFiles(PR));
    expect(files).toHaveLength(all.length);
    expect(files.map((f) => f.path)).toEqual(all.map((f) => f.filename));
    const pages = github.requests
      .filter((r) => r.path.endsWith('/files'))
      .map((r) => r.query.get('page'));
    expect(pages).toEqual(['1', '2', '3']);
  });

  it('list_files_cap_marks_truncated', async () => {
    const all = Array.from({ length: 150 }, (_, i) => file(i));
    github.route('GET', '/repos/acme/billing/pulls/42/files', (req, res) => {
      const page = Number(req.query.get('page') ?? '1');
      FakeGithub.json(res, 200, all.slice((page - 1) * 100, page * 100));
    });
    const capped = await collectChangedFiles(provider.listChangedFiles(PR), 120);
    expect(capped.truncated).toBe(true);
    expect(capped.files).toHaveLength(120);
    const exact = await collectChangedFiles(provider.listChangedFiles(PR), 150);
    expect(exact.truncated).toBe(false);
  });

  it('a missing repository surfaces as ProviderError{not_found}', async () => {
    await expect(provider.getRepository(PR)).rejects.toMatchObject({
      name: 'ProviderError',
      kind: 'not_found',
    });
    await expect(provider.getRepository(PR)).rejects.toBeInstanceOf(ProviderError);
  });

  it('token_scoped_read_only_single_repo', async () => {
    const credential = await provider.issueCloneCredential(PR, 3600, { providerRepoId: '700001' });
    expect(credential.token.reveal()).toMatch(/^ghs_fake/);
    expect(String(credential.token)).toBe('[redacted]');
    const mint = github.requests.find((r) => r.path === '/app/installations/77/access_tokens');
    expect(mint?.body).toEqual({ repository_ids: [700001], permissions: { contents: 'read' } });
  });

  it('getCommit maps parents and author', async () => {
    github.route('GET', '/repos/acme/billing/commits/abc', (_req, res) =>
      FakeGithub.json(res, 200, {
        sha: 'abc',
        commit: { message: 'm', author: { date: '2026-10-01T00:00:00Z' } },
        author: { login: 'octo-dev' },
        parents: [{ sha: 'p1' }],
      }),
    );
    await expect(provider.getCommit(PR, 'abc')).resolves.toEqual({
      sha: 'abc',
      message: 'm',
      authorLogin: 'octo-dev',
      authoredAt: '2026-10-01T00:00:00Z',
      parents: ['p1'],
    });
  });
});
