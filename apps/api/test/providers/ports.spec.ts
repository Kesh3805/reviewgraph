import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { inspect } from 'node:util';
import { Secret } from '../../src/common/secret';
import {
  PROVIDER_RESOLVER,
  ProviderError,
  type CloneCredential,
  type PublishRequest,
  type RepositoryProvider,
  type ReviewEvent,
  type ReviewPublisher,
} from '../../src/providers/ports';
import {
  ProviderRegistry,
  UnknownProviderError,
  repositoryProviderToken,
  reviewPublisherToken,
} from '../../src/providers/provider.registry';

const API_ROOT = resolve(__dirname, '../..');

function lint(filePath: string, source: string): { ruleId: string | null; message: string }[] {
  const res = spawnSync(
    process.execPath,
    [
      resolve(API_ROOT, 'node_modules/eslint/bin/eslint.js'),
      '--stdin',
      '--stdin-filename',
      filePath,
      '--format',
      'json',
    ],
    { cwd: API_ROOT, input: source, encoding: 'utf8' },
  );
  const parsed = JSON.parse(res.stdout) as {
    messages: { ruleId: string | null; message: string }[];
  }[];
  return parsed[0]?.messages ?? [];
}

describe('provider ports', () => {
  it('eslint_blocks_octokit_in_domain_modules', () => {
    const planted = "import { Octokit } from '@octokit/rest';\nexport const o = Octokit;\n";
    for (const dir of ['reviews', 'publisher', 'findings', 'repositories', 'webhooks']) {
      const messages = lint(`src/${dir}/planted.ts`, planted);
      expect(messages.map((m) => m.ruleId)).toContain('no-restricted-imports');
    }
    // Allowed inside the GitHub provider module.
    expect(lint('src/providers/github/ok.ts', planted)).toEqual([]);
  });

  it('eslint_blocks_provider_implementations_in_domain_modules', () => {
    const impl = "import { x } from '../providers/github/app-auth.service';\nexport const y = x;\n";
    const registry =
      "import { ProviderRegistry } from '../providers/provider.registry';\nexport const r = ProviderRegistry;\n";
    const ports =
      "import type { RepositoryProvider } from '../providers/ports';\nexport type R = RepositoryProvider;\n";
    const portsDeep =
      "import type { PrRef } from '../providers/ports/types';\nexport type R = PrRef;\n";
    expect(lint('src/publisher/x.ts', impl).map((m) => m.ruleId)).toContain(
      'no-restricted-imports',
    );
    expect(lint('src/reviews/x.ts', registry).map((m) => m.ruleId)).toContain(
      'no-restricted-imports',
    );
    expect(lint('src/findings/x.ts', ports)).toEqual([]);
    expect(lint('src/repositories/x.ts', portsDeep)).toEqual([]);
  });

  it('review_event_type_is_comment_only', () => {
    const ok: ReviewEvent = 'COMMENT';
    // @ts-expect-error APPROVE is not a ReviewEvent (INV-011/012)
    const approve: ReviewEvent = 'APPROVE';
    // @ts-expect-error REQUEST_CHANGES is not a ReviewEvent
    const changes: ReviewEvent = 'REQUEST_CHANGES';
    // @ts-expect-error a PublishRequest cannot carry another event
    const req: Pick<PublishRequest, 'event'> = { event: 'APPROVE' };
    expect([ok, approve, changes, req.event]).toHaveLength(4);
  });

  it('secret_tojson_redacted', () => {
    const credential: CloneCredential = {
      token: new Secret('ghs_supersecret'),
      expiresAt: new Date(0),
      repo: { provider: 'github', installationId: '1', owner: 'o', name: 'r' },
    };
    const text = [
      JSON.stringify(credential),
      String(credential.token),
      `${credential.token}`,
      inspect(credential, { depth: 5 }),
    ].join('\n');
    expect(text).not.toContain('ghs_supersecret');
    expect(JSON.parse(JSON.stringify(credential)).token).toBe('[redacted]');
    expect(credential.token.reveal()).toBe('ghs_supersecret');
  });

  it('registry_resolves_by_provider_kind', () => {
    const registry = new ProviderRegistry();
    const repository = { kind: 'github' } as RepositoryProvider;
    const publisher = {} as ReviewPublisher;
    registry.register('github', { repository, publisher });
    expect(registry.repository('github')).toBe(repository);
    expect(registry.publisher('github')).toBe(publisher);
    expect(registry.kinds()).toEqual(['github']);
    expect(() => registry.repository('gitlab')).toThrow(UnknownProviderError);
    expect(() => registry.publisher('bitbucket')).toThrow(UnknownProviderError);
    expect(() => registry.register('github', { repository, publisher })).toThrow(/already/);
    expect(repositoryProviderToken('github')).toBe(repositoryProviderToken('github'));
    expect(repositoryProviderToken('github')).not.toBe(reviewPublisherToken('github'));
    expect(typeof PROVIDER_RESOLVER).toBe('symbol');
  });

  it('provider_error_carries_kind_and_retry_hint', () => {
    const err = new ProviderError('rate_limited', 'slow down', { retryAfterMs: 1500 });
    expect(err.kind).toBe('rate_limited');
    expect(err.retryAfterMs).toBe(1500);
    expect(err.retryable).toBe(true);
    expect(new ProviderError('forbidden', 'no').retryable).toBe(false);
  });
});
