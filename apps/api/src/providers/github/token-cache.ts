import { createHash } from 'node:crypto';
import { Logger } from '@nestjs/common';
import type { Redis } from 'ioredis';
import { incCounter } from '../../common/metrics';
import { Secret } from '../../common/secret';
import { DEFAULT_TOKEN_KEY_ID, TokenCipher } from '../../security/token-cipher';

/** Which permissions/repositories an installation token is limited to. Unscoped = `{}`. */
export interface TokenScope {
  repositoryIds?: number[];
  permissions?: Record<string, 'read' | 'write'>;
}

export interface CachedToken {
  token: Secret<string>;
  expiresAt: Date;
}

export const REFRESH_MARGIN_SECONDS = 10 * 60;
export const MAX_CACHE_TTL_SECONDS = 50 * 60;
export const MINT_LOCK_TTL_SECONDS = 10;

type CacheRedis = Pick<Redis, 'get' | 'set' | 'del'>;

/** Stable short hash of a scope: same scope, same key, regardless of key order. */
export function scopeHash(scope: TokenScope): string {
  const canonical = JSON.stringify({
    permissions: Object.entries(scope.permissions ?? {}).sort(([a], [b]) => a.localeCompare(b)),
    repositoryIds: [...(scope.repositoryIds ?? [])].sort((a, b) => a - b),
  });
  return createHash('sha256').update(canonical).digest('hex').slice(0, 16);
}

export const tokenCacheKey = (installationId: string, hash: string): string =>
  `rg:gh:itok:${installationId}:${hash}`;
/** Associated data of a cached value: binds it to its installation and cache key (SEC-005). */
export const tokenCacheAad = (installationId: string, redisKey: string): string =>
  `installation:${installationId}|${redisKey}`;
export const mintLockKey = (installationId: string): string => `rg:gh:itok-lock:${installationId}`;

/** `min(expires_at - now - 10min, 50min)` in whole seconds; <= 0 means do not cache. */
export function cacheTtlSeconds(expiresAt: Date, nowMs: number): number {
  const remaining = Math.floor((expiresAt.getTime() - nowMs) / 1000) - REFRESH_MARGIN_SECONDS;
  return Math.min(remaining, MAX_CACHE_TTL_SECONDS);
}

/**
 * Installation-token cache in Redis, AES-256-GCM encrypted with `TOKEN_CACHE_KEY` through
 * {@link TokenCipher} (value `{kid, iv, ciphertext, tag}`; the installation id and Redis key are
 * bound as AAD). Tokens are never persisted anywhere else. Every Redis or decrypt failure degrades
 * to a cache miss: availability over efficiency.
 */
export class InstallationTokenCache {
  private readonly logger = new Logger(InstallationTokenCache.name);
  private readonly cipher: TokenCipher;

  constructor(
    private readonly redis: CacheRedis,
    key: TokenCipher | Buffer,
    private readonly now: () => number = Date.now,
  ) {
    this.cipher =
      key instanceof TokenCipher ? key : new TokenCipher({ kid: DEFAULT_TOKEN_KEY_ID, key });
  }

  async get(installationId: string, hash: string): Promise<CachedToken | null> {
    const redisKey = tokenCacheKey(installationId, hash);
    let raw: string | null;
    try {
      raw = await this.redis.get(redisKey);
    } catch (err) {
      this.warn('cache read failed; treating as miss', err);
      return null;
    }
    if (!raw) return null;
    try {
      const parsed = JSON.parse(
        this.cipher.decrypt(raw, tokenCacheAad(installationId, redisKey)),
      ) as {
        token: string;
        expires_at: string;
      };
      return { token: new Secret(parsed.token), expiresAt: new Date(parsed.expires_at) };
    } catch (err) {
      incCounter('token_cache_decrypt_failures_total');
      this.warn('cache value could not be decrypted; treating as miss', err);
      // A corrupt or foreign value must not be served again.
      await this.redis.del(redisKey).catch(() => undefined);
      return null;
    }
  }

  async put(installationId: string, hash: string, token: CachedToken): Promise<void> {
    const ttl = cacheTtlSeconds(token.expiresAt, this.now());
    if (ttl <= 0) return;
    const redisKey = tokenCacheKey(installationId, hash);
    try {
      const value = this.cipher.encrypt(
        JSON.stringify({ token: token.token.reveal(), expires_at: token.expiresAt.toISOString() }),
        tokenCacheAad(installationId, redisKey),
      );
      await this.redis.set(redisKey, value, 'EX', ttl);
    } catch (err) {
      this.warn('cache write failed; continuing without cache', err);
    }
  }

  async evict(installationId: string, hash: string): Promise<void> {
    try {
      await this.redis.del(tokenCacheKey(installationId, hash));
    } catch (err) {
      this.warn('cache evict failed', err);
    }
  }

  /** Cross-process mint lock (`SET NX EX 10`). Resolves true when Redis is unavailable. */
  async tryLock(installationId: string): Promise<boolean> {
    try {
      return (
        (await this.redis.set(
          mintLockKey(installationId),
          '1',
          'EX',
          MINT_LOCK_TTL_SECONDS,
          'NX',
        )) === 'OK'
      );
    } catch (err) {
      this.warn('mint lock unavailable; minting without it', err);
      return true;
    }
  }

  async unlock(installationId: string): Promise<void> {
    try {
      await this.redis.del(mintLockKey(installationId));
    } catch {
      // The lock expires on its own after MINT_LOCK_TTL_SECONDS.
    }
  }

  private warn(message: string, err: unknown): void {
    // Only the error class: redis errors can embed connection strings.
    this.logger.warn(`${message} (${err instanceof Error ? err.name : 'error'})`);
  }
}
