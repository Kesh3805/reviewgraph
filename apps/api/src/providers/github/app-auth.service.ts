import { createPrivateKey, type KeyObject } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { Logger } from '@nestjs/common';
import { SpanStatusCode, trace } from '@opentelemetry/api';
import { SignJWT } from 'jose';
import { incCounter, setGauge } from '../../common/metrics';
import { Secret } from '../../common/secret';
import { TRACER_NAME } from '../../telemetry/tracer.service';
import { ProviderError } from '../ports';
import { toProviderError } from './errors';
import { createOctokit, type GithubOctokit } from './octokit.factory';
import { scopeHash, type InstallationTokenCache, type TokenScope } from './token-cache';

export const APP_JWT_BACKDATE_SECONDS = 60;
export const APP_JWT_LIFETIME_SECONDS = 540;
/** The App JWT is reused in-process for 8 minutes (its lifetime is 9 after backdating). */
export const APP_JWT_CACHE_SECONDS = 8 * 60;
const LOCK_WAIT_MS = 10_000;
const LOCK_POLL_MS = 100;

export interface GithubAppAuthOptions {
  appId: string;
  privateKey: KeyObject;
  /** `GITHUB_API_URL`. */
  apiUrl: string;
  cache: InstallationTokenCache;
  fetch?: typeof fetch;
  now?: () => number;
  /** Disable Octokit's retry plugin (tests asserting exact request counts). */
  retries?: boolean;
  /** Poll interval while waiting for another process's mint (tests shorten it). */
  lockPollMs?: number;
}

export interface InstallationToken {
  token: Secret<string>;
  expiresAt: Date;
  /** True when served from Redis (a 401 then means the cached token went stale). */
  fromCache: boolean;
}

/** The unscoped token used for general API calls. */
export const UNSCOPED: TokenScope = {};

/**
 * Parses the App private key. Failure messages never include key material, so a bad value
 * fails the boot without writing the PEM anywhere.
 */
export function parsePrivateKey(pem: string): KeyObject {
  try {
    const key = createPrivateKey(pem.replace(/\\n/g, '\n'));
    if (key.asymmetricKeyType !== 'rsa') throw new Error('not rsa');
    return key;
  } catch {
    throw new Error('GitHub App private key could not be parsed (expected an RSA PEM)');
  }
}

/** Loads the key from `GITHUB_APP_PRIVATE_KEY` or `GITHUB_APP_PRIVATE_KEY_FILE`. */
export function loadPrivateKey(env: {
  GITHUB_APP_PRIVATE_KEY?: string;
  GITHUB_APP_PRIVATE_KEY_FILE?: string;
}): KeyObject {
  if (env.GITHUB_APP_PRIVATE_KEY) return parsePrivateKey(env.GITHUB_APP_PRIVATE_KEY);
  if (env.GITHUB_APP_PRIVATE_KEY_FILE) {
    let pem: string;
    try {
      pem = readFileSync(env.GITHUB_APP_PRIVATE_KEY_FILE, 'utf8');
    } catch {
      throw new Error('GitHub App private key file could not be read');
    }
    return parsePrivateKey(pem);
  }
  throw new Error('GitHub App private key is not configured');
}

/**
 * GitHub App authentication (GH-001): the App JWT, installation tokens (single-flight, cached
 * encrypted in Redis, never persisted) and per-installation Octokit clients.
 */
export class GithubAppAuth {
  private readonly logger = new Logger(GithubAppAuth.name);
  private readonly now: () => number;
  private readonly inflight = new Map<string, Promise<InstallationToken>>();
  private jwt?: { token: Secret<string>; refreshAtMs: number };

  constructor(private readonly opts: GithubAppAuthOptions) {
    this.now = opts.now ?? Date.now;
  }

  /** RS256 App JWT: `iat = now-60`, `exp = now+540`, cached in-process for 8 minutes. */
  async appJwt(): Promise<Secret<string>> {
    const nowMs = this.now();
    if (this.jwt && nowMs < this.jwt.refreshAtMs) return this.jwt.token;
    const nowS = Math.floor(nowMs / 1000);
    const token = await new SignJWT({})
      .setProtectedHeader({ alg: 'RS256' })
      .setIssuer(this.opts.appId)
      .setIssuedAt(nowS - APP_JWT_BACKDATE_SECONDS)
      .setExpirationTime(nowS + APP_JWT_LIFETIME_SECONDS)
      .sign(this.opts.privateKey);
    this.jwt = { token: new Secret(token), refreshAtMs: nowMs + APP_JWT_CACHE_SECONDS * 1000 };
    return this.jwt.token;
  }

  /** The permissions GitHub reports for this App (`GET /app`), used by the boot check (GH-010). */
  async getAppPermissions(): Promise<Record<string, string>> {
    try {
      const octokit = await this.appOctokit();
      const res = await octokit.request('GET /app');
      return { ...((res.data as { permissions?: Record<string, string> }).permissions ?? {}) };
    } catch (err) {
      throw toProviderError(err, 'app permission lookup');
    }
  }

  /** An Octokit authenticated as the App itself (for `/app/**` endpoints). */
  async appOctokit(): Promise<GithubOctokit> {
    return createOctokit({
      baseUrl: this.opts.apiUrl,
      auth: (await this.appJwt()).reveal(),
      fetch: this.opts.fetch,
      retries: this.opts.retries,
    });
  }

  /**
   * Returns a valid installation token, minting at most once per cache key across concurrent
   * callers (in-process promise map plus the Redis mint lock across processes).
   */
  getInstallationToken(
    installationId: string,
    scope: TokenScope = UNSCOPED,
  ): Promise<InstallationToken> {
    const hash = scopeHash(scope);
    const flightKey = `${installationId}:${hash}`;
    const existing = this.inflight.get(flightKey);
    if (existing) return existing;
    const promise = this.resolveToken(installationId, scope, hash).finally(() => {
      this.inflight.delete(flightKey);
    });
    this.inflight.set(flightKey, promise);
    return promise;
  }

  /** Drops a cached token (after GitHub rejected it). */
  evict(installationId: string, scope: TokenScope = UNSCOPED): Promise<void> {
    return this.opts.cache.evict(installationId, scopeHash(scope));
  }

  /**
   * An Octokit for one installation. The token is resolved up front (so 20 concurrent callers
   * mint one token) and again per request from the cache; a 401 on a cached token evicts it and
   * retries once with a fresh mint.
   */
  async getOctokit(installationId: string): Promise<GithubOctokit> {
    await this.getInstallationToken(installationId);
    const octokit = createOctokit({
      baseUrl: this.opts.apiUrl,
      fetch: this.opts.fetch,
      retries: this.opts.retries,
    });
    octokit.hook.wrap('request', async (request, options) => {
      const send = async (token: InstallationToken) => {
        // Inner hooks are bound to this same options object, so mutate it in place.
        options.headers.authorization = `token ${token.token.reveal()}`;
        const response = await request(options);
        const remaining = Number(response.headers['x-ratelimit-remaining']);
        if (Number.isFinite(remaining)) {
          setGauge('github_rate_limit_remaining', remaining, { installation: installationId });
        }
        return response;
      };
      const token = await this.getInstallationToken(installationId);
      try {
        return await send(token);
      } catch (err) {
        if ((err as { status?: number }).status === 401 && token.fromCache) {
          await this.evict(installationId);
          return send(await this.getInstallationToken(installationId));
        }
        throw err;
      }
    });
    return octokit;
  }

  private async resolveToken(
    installationId: string,
    scope: TokenScope,
    hash: string,
  ): Promise<InstallationToken> {
    const { cache } = this.opts;
    const deadline = this.now() + LOCK_WAIT_MS;
    let missCounted = false;
    for (;;) {
      const cached = await cache.get(installationId, hash);
      if (cached && cached.expiresAt.getTime() > this.now()) {
        incCounter('github_token_cache_hits_total');
        return { ...cached, fromCache: true };
      }
      if (!missCounted) {
        incCounter('github_token_cache_misses_total');
        missCounted = true;
      }
      if (await cache.tryLock(installationId)) {
        try {
          // Another process may have finished minting between our read and the lock.
          const recheck = await cache.get(installationId, hash);
          if (recheck && recheck.expiresAt.getTime() > this.now()) {
            return { ...recheck, fromCache: true };
          }
          const minted = await this.mint(installationId, scope);
          await cache.put(installationId, hash, minted);
          return { ...minted, fromCache: false };
        } finally {
          await cache.unlock(installationId);
        }
      }
      if (this.now() >= deadline) {
        // The lock holder is stuck: mint anyway rather than fail the caller.
        const minted = await this.mint(installationId, scope);
        await cache.put(installationId, hash, minted);
        return { ...minted, fromCache: false };
      }
      await new Promise((resolve) => setTimeout(resolve, this.opts.lockPollMs ?? LOCK_POLL_MS));
    }
  }

  private async mint(
    installationId: string,
    scope: TokenScope,
  ): Promise<{ token: Secret<string>; expiresAt: Date }> {
    return trace.getTracer(TRACER_NAME).startActiveSpan('github_token_mint', async (span) => {
      try {
        span.setAttribute('github.installation_id', installationId);
        const octokit = await this.appOctokit();
        const response = await octokit.request(
          'POST /app/installations/{installation_id}/access_tokens',
          {
            installation_id: Number(installationId),
            ...(scope.repositoryIds ? { repository_ids: scope.repositoryIds } : {}),
            ...(scope.permissions ? { permissions: scope.permissions } : {}),
          },
        );
        const data = response.data as { token?: string; expires_at?: string };
        if (!data.token || !data.expires_at) {
          throw new ProviderError('transient', 'token mint returned an unexpected response');
        }
        return { token: new Secret(data.token), expiresAt: new Date(data.expires_at) };
      } catch (err) {
        const error = toProviderError(err, 'installation token mint');
        span.recordException(new Error(error.message));
        span.setStatus({ code: SpanStatusCode.ERROR, message: error.message });
        this.logger.warn(`${error.message} installation=${installationId}`);
        throw error;
      } finally {
        span.end();
      }
    });
  }
}
