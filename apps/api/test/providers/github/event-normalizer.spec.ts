import RedisMock from 'ioredis-mock';
import { counterTotal, resetCounterTotals } from '../../../src/common/metrics';
import { GithubAppAuth } from '../../../src/providers/github/app-auth.service';
import { GithubCommandAcknowledger } from '../../../src/providers/github/command-reaction';
import { GithubEventNormalizer } from '../../../src/providers/github/event-normalizer.service';
import {
  GithubActorPermissions,
  mapGithubPermission,
} from '../../../src/providers/github/permissions';
import { InstallationTokenCache } from '../../../src/providers/github/token-cache';
import {
  ProviderError,
  type ActorPermission,
  type ReviewCommandEvent,
} from '../../../src/providers/ports';
import {
  DEFAULT_REPOSITORY_SETTINGS,
  type RepositorySettings,
} from '../../../src/repositories/repository-settings.port';
import { testConfig } from '../../helpers';
import { fixture } from '../../helpers/fixtures';
import { FakeGithub, generateAppKey } from '../../helpers/fake-github';

function makeNormalizer(opts: {
  permission?: ActorPermission | Error;
  settings?: Partial<RepositorySettings>;
  slug?: string;
}) {
  const lookups: string[] = [];
  const normalizer = new GithubEventNormalizer(
    testConfig({ GITHUB_APP_SLUG: opts.slug ?? 'reviewgraph' }),
    { getSettings: () => Promise.resolve({ ...DEFAULT_REPOSITORY_SETTINGS, ...opts.settings }) },
    {
      getActorPermission: (_ref, login) => {
        lookups.push(login);
        const p = opts.permission ?? 'write';
        return p instanceof Error ? Promise.reject(p) : Promise.resolve(p);
      },
    },
  );
  return { normalizer, lookups };
}

describe('GithubEventNormalizer', () => {
  beforeEach(() => resetCounterTotals());

  it('normalizes an opened PR and applies repository guards', async () => {
    const { normalizer } = makeNormalizer({});
    const opened = fixture('pull_request.opened.json');
    expect(await normalizer.normalize('pull_request', opened, 'd')).toMatchObject({
      type: 'pull_request_head',
      kind: 'opened',
    });
    expect(counterTotal('provider_events_total', { kind: 'opened', outcome: 'normalized' })).toBe(
      1,
    );

    opened.pull_request.draft = true;
    expect(await normalizer.normalize('pull_request', opened, 'd')).toEqual({
      ignored: true,
      reason: 'draft',
    });
    expect(
      counterTotal('provider_events_total', { kind: 'pull_request', outcome: 'ignored_draft' }),
    ).toBe(1);

    const bot = fixture('pull_request.opened.json');
    bot.pull_request.user = { login: 'dependabot[bot]', type: 'Bot' };
    expect(await normalizer.normalize('pull_request', bot, 'd')).toEqual({
      ignored: true,
      reason: 'bot_author',
    });

    const { normalizer: release } = makeNormalizer({ settings: { targetBranches: ['release/*'] } });
    expect(
      await release.normalize('pull_request', fixture('pull_request.opened.json'), 'd'),
    ).toEqual({
      ignored: true,
      reason: 'branch_not_targeted',
    });
  });

  it('closed events pass through regardless of guards so runs are cancelled', async () => {
    const { normalizer } = makeNormalizer({ settings: { targetBranches: ['release/*'] } });
    expect(
      await normalizer.normalize('pull_request', fixture('pull_request.closed.json'), 'd'),
    ).toMatchObject({
      type: 'pull_request_closed',
    });
  });

  it('review_requested only triggers for the configured App bot', async () => {
    const { normalizer } = makeNormalizer({});
    expect(
      await normalizer.normalize(
        'pull_request',
        fixture('pull_request.review_requested.json'),
        'd',
      ),
    ).toMatchObject({ kind: 'review_requested', requestedReviewer: 'reviewgraph[bot]' });
    const { normalizer: other } = makeNormalizer({ slug: 'other-app' });
    expect(
      await other.normalize('pull_request', fixture('pull_request.review_requested.json'), 'd'),
    ).toEqual({ ignored: true, reason: 'reviewer_not_app' });
  });

  it('review_command_requires_write', async () => {
    for (const permission of ['write', 'admin'] as const) {
      const { normalizer, lookups } = makeNormalizer({ permission });
      const result = await normalizer.normalize(
        'issue_comment',
        fixture('issue_comment.created.json'),
        'd',
      );
      expect(result).toEqual({
        type: 'review_command',
        provider: 'github',
        deliveryId: 'd',
        installationId: '4242',
        repo: { provider: 'github', installationId: '4242', owner: 'acme', name: 'billing' },
        pr: {
          provider: 'github',
          installationId: '4242',
          owner: 'acme',
          name: 'billing',
          number: 1347,
        },
        command: 'review',
        commentId: '9001',
        actor: { login: 'maintainer', isBot: false },
      });
      expect(lookups).toEqual(['maintainer']);
    }
    for (const permission of ['read', 'none'] as const) {
      const { normalizer } = makeNormalizer({ permission });
      expect(
        await normalizer.normalize('issue_comment', fixture('issue_comment.created.json'), 'd'),
      ).toEqual({ ignored: true, reason: 'permission_denied' });
    }
  });

  it('a failed permission lookup fails closed', async () => {
    const { normalizer } = makeNormalizer({ permission: new ProviderError('transient', 'down') });
    expect(
      await normalizer.normalize('issue_comment', fixture('issue_comment.created.json'), 'd'),
    ).toEqual({
      ignored: true,
      reason: 'permission_unknown',
    });
  });

  it('draft_with_explicit_command_reviewed: a /review command on a draft PR proceeds', async () => {
    const { normalizer } = makeNormalizer({});
    const payload = fixture('issue_comment.created.json');
    payload.issue.draft = true;
    expect(await normalizer.normalize('issue_comment', payload, 'd')).toMatchObject({
      type: 'review_command',
      command: 'review',
    });
  });

  it('review_cancel_command: cancel works on bot PRs and /review full sets the command', async () => {
    const { normalizer } = makeNormalizer({});
    const cancel = fixture('issue_comment.created.json');
    cancel.comment.body = '/review cancel';
    cancel.issue.user = { login: 'dependabot[bot]', type: 'Bot' };
    expect(await normalizer.normalize('issue_comment', cancel, 'd')).toMatchObject({
      type: 'review_command',
      command: 'cancel',
    });
    const review = fixture('issue_comment.created.json');
    review.issue.user = { login: 'dependabot[bot]', type: 'Bot' };
    expect(await normalizer.normalize('issue_comment', review, 'd')).toEqual({
      ignored: true,
      reason: 'bot_author',
    });
    const full = fixture('issue_comment.created.json');
    full.comment.body = '/review full';
    expect(await normalizer.normalize('issue_comment', full, 'd')).toMatchObject({
      command: 'full',
    });
  });

  it('malformed_payload_ignored is counted', async () => {
    const { normalizer } = makeNormalizer({});
    expect(await normalizer.normalize('pull_request', { action: 'opened' }, 'd')).toEqual({
      ignored: true,
      reason: 'malformed',
    });
    expect(
      counterTotal('provider_events_total', { kind: 'pull_request', outcome: 'ignored_malformed' }),
    ).toBe(1);
  });
});

describe('GitHub permission lookup and command acknowledgement (fake GitHub)', () => {
  const key = generateAppKey();
  let github: FakeGithub;
  let auth: GithubAppAuth;

  beforeAll(async () => {
    github = new FakeGithub(key.publicKey);
    await github.start();
    auth = new GithubAppAuth({
      appId: '1',
      privateKey: key.privateKey,
      apiUrl: github.url,
      cache: new InstallationTokenCache(new RedisMock(), Buffer.alloc(32, 3)),
      retries: false,
    });
  });
  afterAll(async () => {
    await github.stop();
  });

  it('maps GitHub collaborator permissions', async () => {
    github.route('GET', '/repos/acme/billing/collaborators/maintainer/permission', (_req, res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          permission: 'write',
          role_name: 'maintain',
          user: { login: 'maintainer' },
        }),
      );
    });
    github.route('GET', '/repos/acme/billing/collaborators/stranger/permission', (_req, res) => {
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ permission: 'none', role_name: 'none' }));
    });
    const lookup = new GithubActorPermissions(auth);
    const ref = {
      provider: 'github' as const,
      installationId: '4242',
      owner: 'acme',
      name: 'billing',
    };
    expect(await lookup.getActorPermission(ref, 'maintainer')).toBe('write');
    expect(await lookup.getActorPermission(ref, 'stranger')).toBe('none');
    await expect(lookup.getActorPermission(ref, 'ghost')).rejects.toMatchObject({
      kind: 'not_found',
    });
    await expect(
      new GithubActorPermissions(null).getActorPermission(ref, 'x'),
    ).rejects.toBeInstanceOf(ProviderError);
    expect(mapGithubPermission({ permission: 'admin', role_name: 'admin' })).toBe('admin');
    expect(mapGithubPermission({ permission: 'write', role_name: 'triage' })).toBe('read');
    expect(mapGithubPermission({ permission: 'read' })).toBe('read');
    expect(mapGithubPermission({})).toBe('none');
  });

  it('acknowledges a command with an eyes reaction and no text comment', async () => {
    let body: unknown;
    github.route('POST', '/repos/acme/billing/issues/comments/9001/reactions', (req, res) => {
      body = req.body;
      res.writeHead(201, { 'content-type': 'application/json' });
      res.end('{"id":1,"content":"eyes"}');
    });
    const before = github.requests.length;
    const event = {
      type: 'review_command',
      provider: 'github',
      deliveryId: 'd',
      installationId: '4242',
      repo: { provider: 'github', installationId: '4242', owner: 'acme', name: 'billing' },
      pr: {
        provider: 'github',
        installationId: '4242',
        owner: 'acme',
        name: 'billing',
        number: 1347,
      },
      command: 'review',
      commentId: '9001',
      actor: { login: 'maintainer', isBot: false },
    } satisfies ReviewCommandEvent;
    await new GithubCommandAcknowledger(auth).acknowledge(event);
    expect(body).toEqual({ content: 'eyes' });
    const paths = github.requests.slice(before).map((r) => `${r.method} ${r.path}`);
    expect(paths).not.toContain('POST /repos/acme/billing/issues/1347/comments');
    await expect(new GithubCommandAcknowledger(null).acknowledge(event)).resolves.toBeUndefined();
  });
});
