import RedisMock from 'ioredis-mock';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import { Secret } from '../../src/common/secret';
import { GithubAppAuth } from '../../src/providers/github/app-auth.service';
import {
  InstallationTokenCache,
  MAX_CACHE_TTL_SECONDS,
  scopeHash,
  tokenCacheAad,
  tokenCacheKey,
} from '../../src/providers/github/token-cache';
import {
  TokenCipher,
  TokenCipherError,
  isValidPreviousKeysSpec,
  isValidTokenKeySpec,
  parseTokenKey,
} from '../../src/security/token-cipher';
import { testConfig } from '../helpers';
import { FakeGithub, generateAppKey } from '../helpers/fake-github';

const KEY_A = Buffer.alloc(32, 3);
const KEY_B = Buffer.alloc(32, 4);
const APP_KEY = generateAppKey();
const HASH = scopeHash({});

describe('SEC-005 token cache encryption', () => {
  let redis: InstanceType<typeof RedisMock>;

  beforeEach(async () => {
    redis = new RedisMock();
    await redis.flushall();
    resetCounterTotals();
  });

  it('token_cache_value_is_ciphertext_in_redis', async () => {
    const cipher = new TokenCipher({ kid: 'k7', key: KEY_A });
    const cache = new InstallationTokenCache(redis, cipher);
    const token = 'ghs_sentinelTOKEN0123456789abcdefABCDEF';
    await cache.put('101', HASH, {
      token: new Secret(token),
      expiresAt: new Date(Date.now() + 3600_000),
    });
    const raw = await redis.get(tokenCacheKey('101', HASH));
    expect(raw).toBeTruthy();
    const stored = JSON.parse(raw!) as Record<string, string>;
    expect(Object.keys(stored).sort()).toEqual(['ciphertext', 'iv', 'kid', 'tag']);
    expect(stored.kid).toBe('k7');
    for (const key of await redis.keys('*')) {
      const value = (await redis.get(key)) ?? '';
      expect(`${key}${value}`).not.toContain(token);
      expect(`${key}${value}`).not.toContain(Buffer.from(token).toString('base64'));
    }
    // The Redis key carries the installation id only, never the token.
    expect(tokenCacheKey('101', HASH)).toMatch(/^rg:gh:itok:101:[0-9a-f]{16}$/);
    expect((await cache.get('101', HASH))?.token.reveal()).toBe(token);
  });

  it('token_cache_ttl_below_expiry', async () => {
    const now = 1_900_000_000_000;
    const cache = new InstallationTokenCache(redis, KEY_A, () => now);
    const put = async (lifetimeSeconds: number) => {
      await redis.flushall();
      await cache.put('7', HASH, {
        token: new Secret('ghs_x'),
        expiresAt: new Date(now + lifetimeSeconds * 1000),
      });
      return redis.ttl(tokenCacheKey('7', HASH));
    };
    for (const lifetime of [3600, 7200, 1800, 900, 661]) {
      const ttl = await put(lifetime);
      expect(ttl).toBeLessThanOrEqual(MAX_CACHE_TTL_SECONDS);
      expect(ttl).toBeLessThan(lifetime - 60);
    }
    // A token about to expire is not cached at all.
    expect(await put(300)).toBe(-2);
  });

  it('swapped_cache_value_fails_aad_check', async () => {
    const cache = new InstallationTokenCache(redis, KEY_A);
    await cache.put('111', HASH, {
      token: new Secret('ghs_for_installation_111'),
      expiresAt: new Date(Date.now() + 3600_000),
    });
    // An attacker with Redis write access copies installation 111's value to 222.
    const stolen = await redis.get(tokenCacheKey('111', HASH));
    await redis.set(tokenCacheKey('222', HASH), stolen!);
    expect(await cache.get('222', HASH)).toBeNull();
    expect(counterTotal('token_cache_decrypt_failures_total')).toBe(1);
    // The poisoned value is dropped, the legitimate one still works.
    expect(await redis.get(tokenCacheKey('222', HASH))).toBeNull();
    expect((await cache.get('111', HASH))?.token.reveal()).toBe('ghs_for_installation_111');

    const cipher = new TokenCipher({ kid: 'k1', key: KEY_A });
    expect(() =>
      cipher.decrypt(stolen!, tokenCacheAad('222', tokenCacheKey('222', HASH))),
    ).toThrow(TokenCipherError);
  });

  it('rotation: values under a previous kid decrypt, new values use the current kid', async () => {
    const old = new InstallationTokenCache(redis, new TokenCipher({ kid: 'old', key: KEY_A }));
    await old.put('9', HASH, {
      token: new Secret('ghs_written_before_rotation'),
      expiresAt: new Date(Date.now() + 3600_000),
    });
    const rotated = new InstallationTokenCache(
      redis,
      TokenCipher.fromConfig(`new:${KEY_B.toString('base64')}`, `old:${KEY_A.toString('base64')}`),
    );
    expect((await rotated.get('9', HASH))?.token.reveal()).toBe('ghs_written_before_rotation');
    await rotated.put('10', HASH, {
      token: new Secret('ghs_after'),
      expiresAt: new Date(Date.now() + 3600_000),
    });
    expect((JSON.parse((await redis.get(tokenCacheKey('10', HASH)))!) as { kid: string }).kid).toBe(
      'new',
    );
    // Without the previous key the old value is a miss (re-mint), not an error.
    const forgot = new InstallationTokenCache(redis, new TokenCipher({ kid: 'new', key: KEY_B }));
    expect(await forgot.get('9', HASH)).toBeNull();
  });

  it('validates key specs without echoing them', () => {
    const b64 = KEY_A.toString('base64');
    expect(isValidTokenKeySpec(b64)).toBe(true);
    expect(isValidTokenKeySpec(`k2:${b64}`)).toBe(true);
    expect(isValidTokenKeySpec(Buffer.alloc(16).toString('base64'))).toBe(false);
    expect(isValidPreviousKeysSpec(`a:${b64},b:${KEY_B.toString('base64')}`)).toBe(true);
    expect(isValidPreviousKeysSpec(b64)).toBe(false);
    expect(parseTokenKey(b64).kid).toBe('k1');
    expect(() => new TokenCipher({ kid: 'x', key: KEY_A }, [{ kid: 'x', key: KEY_B }])).toThrow(
      /duplicate/,
    );
    let message = '';
    try {
      testConfig({ TOKEN_CACHE_KEY_PREVIOUS: 'zzzz-not-a-key' });
    } catch (err) {
      message = (err as Error).message;
    }
    expect(message).toContain('TOKEN_CACHE_KEY_PREVIOUS');
    expect(message).not.toContain('zzzz-not-a-key');
  });

  describe('with the fake GitHub', () => {
    let github: FakeGithub;
    beforeAll(async () => {
      github = new FakeGithub(APP_KEY.publicKey);
      await github.start();
    });
    afterAll(() => github.stop());

    it('single_flight_mint_per_installation', async () => {
      github.mintCount = 0;
      github.mintDelayMs = 40;
      const cache = new InstallationTokenCache(redis, KEY_A);
      const auth = (processId: number) =>
        new GithubAppAuth({
          appId: `app-${processId}`,
          privateKey: APP_KEY.privateKey,
          apiUrl: github.url,
          cache,
          retries: false,
          lockPollMs: 5,
        });
      // Two "processes" sharing Redis, ten concurrent callers each.
      const a = auth(1);
      const b = auth(2);
      const tokens = await Promise.all(
        Array.from({ length: 20 }, (_, i) => (i % 2 ? a : b).getInstallationToken('55')),
      );
      expect(new Set(tokens.map((t) => t.token.reveal())).size).toBe(1);
      expect(github.mintCount).toBe(1);
      expect(counterTotal('github_token_mint_total', { outcome: 'success' })).toBe(1);
      github.mintDelayMs = 0;
    });
  });
});
