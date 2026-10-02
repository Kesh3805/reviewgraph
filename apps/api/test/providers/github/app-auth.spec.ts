import { Logger } from '@nestjs/common';
import { decodeJwt, decodeProtectedHeader, jwtVerify } from 'jose';
import RedisMock from 'ioredis-mock';
import { aeadDecrypt, aeadEncrypt, AeadError } from '../../../src/common/crypto/aead';
import { counterTotal, gaugeValue, resetCounterTotals } from '../../../src/common/metrics';
import {
  GithubAppAuth,
  loadPrivateKey,
  parsePrivateKey,
} from '../../../src/providers/github/app-auth.service';
import { createGithubAppAuth } from '../../../src/providers/github/github.module';
import {
  InstallationTokenCache,
  cacheTtlSeconds,
  scopeHash,
  tokenCacheKey,
} from '../../../src/providers/github/token-cache';
import { ProviderError } from '../../../src/providers/ports';
import { testConfig } from '../../helpers';
import { FakeGithub, generateAppKey } from '../../helpers/fake-github';

const CACHE_KEY = Buffer.alloc(32, 7);
const INSTALLATION = '4242';
const APP_KEY = generateAppKey();

describe('GitHub App auth', () => {
  let github: FakeGithub;
  let redis: InstanceType<typeof RedisMock>;

  const makeAuth = (extra: Partial<ConstructorParameters<typeof GithubAppAuth>[0]> = {}) =>
    new GithubAppAuth({
      appId: '12345',
      privateKey: APP_KEY.privateKey,
      apiUrl: github.url,
      cache: new InstallationTokenCache(redis, CACHE_KEY),
      retries: false,
      lockPollMs: 5,
      ...extra,
    });

  beforeAll(async () => {
    github = new FakeGithub(APP_KEY.publicKey);
    await github.start();
  });
  afterAll(async () => {
    await github.stop();
  });
  beforeEach(async () => {
    redis = new RedisMock();
    await redis.flushall();
    github.mintCount = 0;
    github.mintDelayMs = 0;
    github.mintFailure = undefined;
    github.tokenLifetimeSeconds = 3600;
    github.requests.length = 0;
    github.revoked.clear();
    github.routes.clear();
    resetCounterTotals();
  });

  it('app_jwt_claims_valid', async () => {
    let now = 1_800_000_000_000;
    const auth = makeAuth({ now: () => now });
    const jwt = (await auth.appJwt()).reveal();
    expect(decodeProtectedHeader(jwt).alg).toBe('RS256');
    const claims = decodeJwt(jwt);
    expect(claims.iss).toBe('12345');
    expect(claims.iat).toBe(now / 1000 - 60);
    expect(claims.exp).toBe(now / 1000 + 540);
    await expect(
      jwtVerify(jwt, APP_KEY.publicKey, {
        algorithms: ['RS256'],
        currentDate: new Date(now),
      }),
    ).resolves.toBeDefined();
    // Cached in-process for 8 minutes, then re-signed.
    now += 7 * 60_000;
    expect((await auth.appJwt()).reveal()).toBe(jwt);
    now += 2 * 60_000;
    expect((await auth.appJwt()).reveal()).not.toBe(jwt);
  });

  it('token_cached_encrypted_not_plaintext', async () => {
    const auth = makeAuth();
    const token = await auth.getInstallationToken(INSTALLATION);
    const secret = token.token.reveal();
    expect(token.fromCache).toBe(false);

    const key = tokenCacheKey(INSTALLATION, scopeHash({}));
    const raw = await redis.get(key);
    expect(raw).toBeTruthy();
    expect(raw).not.toContain(secret);
    expect(raw).not.toContain(secret.slice(4, 14));
    // The whole keyspace holds no plaintext either.
    for (const k of await redis.keys('*'))
      expect(`${k}${await redis.get(k)}`).not.toContain(secret);

    const decrypted = JSON.parse(aeadDecrypt(CACHE_KEY, raw!, key)) as { token: string };
    expect(decrypted.token).toBe(secret);
    // Bound to its key and to TOKEN_CACHE_KEY.
    expect(() => aeadDecrypt(CACHE_KEY, raw!, 'rg:gh:itok:other:key')).toThrow(AeadError);
    expect(() => aeadDecrypt(Buffer.alloc(32, 8), raw!, key)).toThrow(AeadError);

    const again = await auth.getInstallationToken(INSTALLATION);
    expect(again.fromCache).toBe(true);
    expect(again.token.reveal()).toBe(secret);
    expect(github.mintCount).toBe(1);
    expect(counterTotal('github_token_cache_hits_total')).toBe(1);
    expect(counterTotal('github_token_cache_misses_total')).toBe(1);
  });

  it('token_ttl_below_expiry', async () => {
    const key = tokenCacheKey(INSTALLATION, scopeHash({}));
    const auth = makeAuth();
    await auth.getInstallationToken(INSTALLATION);
    const ttl = await redis.ttl(key);
    // 1 h token: min(60-10, 50) = 50 minutes.
    expect(ttl).toBeLessThanOrEqual(50 * 60);
    expect(ttl).toBeGreaterThan(49 * 60);

    await redis.flushall();
    github.tokenLifetimeSeconds = 30 * 60;
    await makeAuth().getInstallationToken(INSTALLATION);
    const shortTtl = await redis.ttl(key);
    expect(shortTtl).toBeLessThanOrEqual(20 * 60);
    expect(shortTtl).toBeGreaterThan(19 * 60);

    // Too close to expiry to be worth caching.
    await redis.flushall();
    github.tokenLifetimeSeconds = 8 * 60;
    await makeAuth().getInstallationToken(INSTALLATION);
    expect(await redis.get(key)).toBeNull();

    const now = Date.parse('2026-01-01T00:00:00Z');
    expect(cacheTtlSeconds(new Date(now + 3600_000), now)).toBe(3000);
    expect(cacheTtlSeconds(new Date(now + 3600_000 * 3), now)).toBe(3000);
    expect(cacheTtlSeconds(new Date(now + 15 * 60_000), now)).toBe(300);
  });

  it('single_flight_concurrent_mint: 20 concurrent getOctokit calls mint one token', async () => {
    github.mintDelayMs = 50;
    const auth = makeAuth();
    const clients = await Promise.all(
      Array.from({ length: 20 }, () => auth.getOctokit(INSTALLATION)),
    );
    expect(clients).toHaveLength(20);
    expect(github.mintCount).toBe(1);
  });

  it('single_flight_concurrent_mint: separate processes share one mint through the Redis lock', async () => {
    github.mintDelayMs = 80;
    const a = makeAuth();
    const b = makeAuth();
    await Promise.all([
      ...Array.from({ length: 5 }, () => a.getInstallationToken(INSTALLATION)),
      ...Array.from({ length: 5 }, () => b.getInstallationToken(INSTALLATION)),
    ]);
    expect(github.mintCount).toBe(1);
  });

  it('mints scoped tokens with repository_ids and permissions, cached per scope', async () => {
    const auth = makeAuth();
    const scope = { repositoryIds: [9], permissions: { contents: 'read' as const } };
    await auth.getInstallationToken(INSTALLATION, scope);
    await auth.getInstallationToken(INSTALLATION);
    expect(github.mintCount).toBe(2);
    const bodies = github.requests.filter((r) => r.method === 'POST').map((r) => r.body);
    expect(bodies).toContainEqual({ repository_ids: [9], permissions: { contents: 'read' } });
    expect(scopeHash(scope)).toBe(
      scopeHash({ permissions: { contents: 'read' }, repositoryIds: [9] }),
    );
    expect(scopeHash(scope)).not.toBe(scopeHash({}));
  });

  it('cached_token_401_evicts_and_retries', async () => {
    github.route('GET', '/repos/o/r', (_req, res) => {
      res.writeHead(200, { 'content-type': 'application/json', 'x-ratelimit-remaining': '4321' });
      res.end(JSON.stringify({ full_name: 'o/r' }));
    });
    const auth = makeAuth();
    const octokit = await auth.getOctokit(INSTALLATION);
    expect((await octokit.request('GET /repos/o/r')).data).toEqual({ full_name: 'o/r' });
    expect(github.mintCount).toBe(1);
    expect(gaugeValue('github_rate_limit_remaining', { installation: INSTALLATION })).toBe(4321);

    // GitHub revokes the cached token behind our back.
    const cached = await auth.getInstallationToken(INSTALLATION);
    github.revoked.add(cached.token.reveal());
    const res = await octokit.request('GET /repos/o/r');
    expect(res.status).toBe(200);
    expect(github.mintCount).toBe(2);
    const calls = github.requests.filter((r) => r.path === '/repos/o/r');
    expect(calls.map((c) => c.headers.authorization)).toHaveLength(3);
    expect(calls[1]?.headers.authorization).toBe(calls[0]?.headers.authorization);
    expect(calls[2]?.headers.authorization).not.toBe(calls[1]?.headers.authorization);
  });

  it('a 401 on a freshly minted token is not retried', async () => {
    github.route('GET', '/repos/o/r', (_req, res) => {
      res.writeHead(401, { 'content-type': 'application/json' });
      res.end('{"message":"Bad credentials"}');
    });
    const auth = makeAuth({ cache: new InstallationTokenCache(failingRedis(), CACHE_KEY) });
    const octokit = await auth.getOctokit(INSTALLATION);
    await expect(octokit.request('GET /repos/o/r')).rejects.toMatchObject({ status: 401 });
  });

  it('redis_down_falls_back: mints without the cache and logs a warning', async () => {
    const warn = jest.spyOn(Logger.prototype, 'warn').mockImplementation(() => undefined);
    try {
      const auth = makeAuth({ cache: new InstallationTokenCache(failingRedis(), CACHE_KEY) });
      const first = await auth.getInstallationToken(INSTALLATION);
      const second = await auth.getInstallationToken(INSTALLATION);
      expect(first.token.reveal()).toMatch(/^ghs_/);
      expect(second.fromCache).toBe(false);
      expect(github.mintCount).toBe(2);
      expect(warn).toHaveBeenCalled();
    } finally {
      warn.mockRestore();
    }
  });

  it('maps mint failures to ProviderError without leaking credentials', async () => {
    github.mintFailure = {
      status: 403,
      headers: { 'retry-after': '30' },
      body: { message: 'secondary rate limit' },
    };
    const auth = makeAuth();
    const err = await auth.getInstallationToken(INSTALLATION).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ProviderError);
    expect((err as ProviderError).kind).toBe('rate_limited');
    expect((err as ProviderError).retryAfterMs).toBe(30_000);

    github.mintFailure = { status: 404, body: { message: 'Not Found' } };
    const notFound = await makeAuth()
      .getInstallationToken('999')
      .catch((e: unknown) => e);
    expect((notFound as ProviderError).kind).toBe('not_found');
    expect(String(notFound)).not.toMatch(/Bearer|eyJ/);
  });

  describe('pem_never_logged', () => {
    it('keeps the PEM out of errors, logs and serialized state', async () => {
      const captured: string[] = [];
      const spies = (['log', 'warn', 'error', 'debug', 'verbose'] as const).map((level) =>
        jest.spyOn(Logger.prototype, level).mockImplementation((...args: unknown[]) => {
          captured.push(args.map(String).join(' '));
        }),
      );
      const stdout = jest.spyOn(process.stdout, 'write').mockImplementation((chunk) => {
        captured.push(String(chunk));
        return true;
      });
      try {
        github.mintFailure = { status: 500, body: { message: 'boom' } };
        const auth = makeAuth();
        const err = await auth.getInstallationToken(INSTALLATION).catch((e: unknown) => e);
        captured.push(String(err), JSON.stringify(auth), JSON.stringify(err));

        const garbage = `-----BEGIN RSA PRIVATE KEY-----\nQUJDREVGR0hJSktMTU5PUA==\n-----END RSA PRIVATE KEY-----`;
        const parseErr = (() => {
          try {
            parsePrivateKey(garbage);
          } catch (e) {
            return e;
          }
        })();
        captured.push(String(parseErr));
        expect(parseErr).toBeInstanceOf(Error);
        expect(() =>
          loadPrivateKey({ GITHUB_APP_PRIVATE_KEY_FILE: '/definitely/missing.pem' }),
        ).toThrow(/could not be read/);
      } finally {
        spies.forEach((s) => s.mockRestore());
        stdout.mockRestore();
      }
      const body = APP_KEY.pem.split('\n').filter((l) => l && !l.startsWith('-----'))[1] ?? '';
      const output = captured.join('\n');
      expect(output).not.toContain('BEGIN');
      expect(output).not.toContain('QUJDREVGR0hJSktMTU5PUA');
      expect(output).not.toContain(body);
    });

    it('fails the boot on an unparsable key and returns null when GitHub is disabled', () => {
      const bad = testConfig({
        GITHUB_ENABLED: 'true',
        GITHUB_APP_ID: '1',
        GITHUB_WEBHOOK_SECRET: 'w',
        GITHUB_CLIENT_ID: 'c',
        GITHUB_CLIENT_SECRET: 's',
        GITHUB_APP_PRIVATE_KEY: 'not a pem',
      });
      expect(() => createGithubAppAuth(bad, new RedisMock())).toThrow(/private key/);
      expect(createGithubAppAuth(testConfig(), new RedisMock())).toBeNull();
      const good = testConfig({
        GITHUB_ENABLED: 'true',
        GITHUB_APP_ID: '1',
        GITHUB_WEBHOOK_SECRET: 'w',
        GITHUB_CLIENT_ID: 'c',
        GITHUB_CLIENT_SECRET: 's',
        GITHUB_APP_PRIVATE_KEY: APP_KEY.pem.replace(/\n/g, '\\n'),
      });
      expect(createGithubAppAuth(good, new RedisMock())).toBeInstanceOf(GithubAppAuth);
    });
  });

  it('aead round trip uses a fresh nonce per message', () => {
    const a = aeadEncrypt(CACHE_KEY, 'same', 'ctx');
    const b = aeadEncrypt(CACHE_KEY, 'same', 'ctx');
    expect(a).not.toBe(b);
    expect(aeadDecrypt(CACHE_KEY, a, 'ctx')).toBe('same');
    expect(() => aeadDecrypt(CACHE_KEY, `${a}x`, 'ctx')).toThrow(AeadError);
    expect(() => aeadDecrypt(CACHE_KEY, 'garbage', 'ctx')).toThrow(AeadError);
    expect(() => aeadEncrypt(Buffer.alloc(16), 'x')).toThrow(AeadError);
  });
});

/** A Redis whose every command rejects, as during an outage. */
function failingRedis(): InstanceType<typeof RedisMock> {
  const down = () => Promise.reject(new Error('redis://user:hunter2@host connection refused'));
  const redis = new RedisMock();
  redis.get = down as never;
  redis.set = down as never;
  redis.del = down as never;
  return redis;
}
