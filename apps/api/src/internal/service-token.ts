import { randomUUID } from 'node:crypto';
import { SignJWT, decodeProtectedHeader, errors as joseErrors, jwtVerify } from 'jose';

/**
 * Service-to-service tokens (API-005): HS256 JWTs, at most 60 s long, signed with a shared
 * secret selected by `kid`. The claim contract is mirrored by the Rust side
 * (`review-core::service_auth`); the golden vectors in
 * `packages/contracts/fixtures/service-token/` pin the wire format for both languages.
 */

export const SERVICE_ISSUERS = ['rg-api', 'rg-worker', 'rg-engine'] as const;
export const SERVICE_AUDIENCES = ['rg-api', 'rg-engine'] as const;
export const SERVICE_SCOPES = ['clone-credentials', 'graph:read'] as const;

export type ServiceIssuer = (typeof SERVICE_ISSUERS)[number];
export type ServiceAudience = (typeof SERVICE_AUDIENCES)[number];
export type ServiceScope = (typeof SERVICE_SCOPES)[number];

export const MAX_TOKEN_LIFETIME_SECONDS = 60;
export const CLOCK_SKEW_SECONDS = 30;
/** Replay-cache retention: lifetime + skew on both sides, rounded up. */
export const JTI_TTL_SECONDS = 120;
export const MIN_KEY_BYTES = 32;

export interface ServiceTokenClaims {
  iss: ServiceIssuer;
  aud: ServiceAudience;
  /** Service instance id. */
  sub: string;
  scope: ServiceScope[];
  org?: string;
  repo?: string;
  iat: number;
  exp: number;
  jti: string;
}

export interface ServiceKey {
  kid: string;
  secret: Buffer;
}

/** Ordered key ring: the first key signs, every key verifies (rotation). */
export class ServiceKeyRing {
  private constructor(private readonly keys: readonly ServiceKey[]) {}

  /** Parses `kid1:base64,kid2:base64`. Error messages never echo key material. */
  static parse(spec: string): ServiceKeyRing {
    const keys: ServiceKey[] = [];
    for (const entry of spec.split(',')) {
      const trimmed = entry.trim();
      if (!trimmed) continue;
      const idx = trimmed.indexOf(':');
      if (idx <= 0) throw new Error('service key entries must look like kid:base64');
      const kid = trimmed.slice(0, idx);
      const secret = Buffer.from(trimmed.slice(idx + 1), 'base64');
      if (!/^[A-Za-z0-9._-]+$/.test(kid)) throw new Error('service key kid has invalid characters');
      if (secret.length < MIN_KEY_BYTES)
        throw new Error(`service key "${kid}" must be at least ${MIN_KEY_BYTES} bytes`);
      if (keys.some((k) => k.kid === kid)) throw new Error(`duplicate service key kid "${kid}"`);
      keys.push({ kid, secret });
    }
    if (keys.length === 0) throw new Error('at least one service key is required');
    return new ServiceKeyRing(keys);
  }

  static from(keys: ServiceKey[]): ServiceKeyRing {
    if (keys.length === 0) throw new Error('at least one service key is required');
    return new ServiceKeyRing(keys);
  }

  get signing(): ServiceKey {
    const first = this.keys[0];
    if (!first) throw new Error('service key ring is empty');
    return first;
  }

  find(kid: string): ServiceKey | undefined {
    return this.keys.find((k) => k.kid === kid);
  }
}

export interface SignOptions {
  iss: ServiceIssuer;
  aud: ServiceAudience;
  sub: string;
  scope: ServiceScope[];
  org?: string;
  repo?: string;
  /** Lifetime in seconds, 1..=60. Default 60. */
  ttlSeconds?: number;
  /** Unix seconds; defaults to the wall clock. */
  now?: number;
  jti?: string;
}

export async function signServiceToken(ring: ServiceKeyRing, opts: SignOptions): Promise<string> {
  const ttl = opts.ttlSeconds ?? MAX_TOKEN_LIFETIME_SECONDS;
  if (ttl < 1 || ttl > MAX_TOKEN_LIFETIME_SECONDS) {
    throw new Error(`service token lifetime must be 1..=${MAX_TOKEN_LIFETIME_SECONDS} seconds`);
  }
  const iat = opts.now ?? Math.floor(Date.now() / 1000);
  const key = ring.signing;
  const payload: Record<string, unknown> = { scope: opts.scope };
  if (opts.org) payload.org = opts.org;
  if (opts.repo) payload.repo = opts.repo;
  return new SignJWT(payload)
    .setProtectedHeader({ alg: 'HS256', typ: 'JWT', kid: key.kid })
    .setIssuer(opts.iss)
    .setAudience(opts.aud)
    .setSubject(opts.sub)
    .setIssuedAt(iat)
    .setExpirationTime(iat + ttl)
    .setJti(opts.jti ?? randomUUID())
    .sign(key.secret);
}

export type ServiceAuthFailureReason =
  | 'missing_token'
  | 'malformed'
  | 'unknown_kid'
  | 'bad_signature'
  | 'expired'
  | 'not_yet_valid'
  | 'wrong_audience'
  | 'wrong_issuer'
  | 'lifetime_too_long'
  | 'invalid_claims'
  | 'replayed';

/** Verification failure. The reason is for logs and metrics only, never for the response body. */
export class ServiceAuthError extends Error {
  constructor(readonly reason: ServiceAuthFailureReason) {
    super(`service token rejected: ${reason}`);
    this.name = 'ServiceAuthError';
  }
}

export interface VerifyOptions {
  audience: ServiceAudience;
  /** Unix seconds; defaults to the wall clock. */
  now?: number;
  allowedIssuers?: readonly ServiceIssuer[];
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Verifies signature, audience, expiry (30 s skew), lifetime bound and claim shape. */
export async function verifyServiceToken(
  ring: ServiceKeyRing,
  token: string,
  opts: VerifyOptions,
): Promise<ServiceTokenClaims> {
  let kid: string | undefined;
  try {
    const header = decodeProtectedHeader(token);
    if (header.alg !== 'HS256') throw new ServiceAuthError('malformed');
    kid = header.kid;
  } catch (err) {
    if (err instanceof ServiceAuthError) throw err;
    throw new ServiceAuthError('malformed');
  }
  const key = kid ? ring.find(kid) : undefined;
  if (!key) throw new ServiceAuthError('unknown_kid');

  let payload;
  try {
    ({ payload } = await jwtVerify(token, key.secret, {
      algorithms: ['HS256'],
      audience: opts.audience,
      clockTolerance: CLOCK_SKEW_SECONDS,
      currentDate: opts.now === undefined ? undefined : new Date(opts.now * 1000),
      requiredClaims: ['iss', 'aud', 'sub', 'iat', 'exp', 'jti'],
    }));
  } catch (err) {
    throw new ServiceAuthError(classify(err));
  }

  const { iss, sub, iat, exp, jti, scope, org, repo } = payload as Record<string, unknown>;
  const issuers: readonly string[] = opts.allowedIssuers ?? SERVICE_ISSUERS;
  if (typeof iss !== 'string' || !issuers.includes(iss)) throw new ServiceAuthError('wrong_issuer');
  if (
    typeof sub !== 'string' ||
    typeof jti !== 'string' ||
    typeof iat !== 'number' ||
    typeof exp !== 'number' ||
    !Array.isArray(scope) ||
    !scope.every(
      (s) => typeof s === 'string' && (SERVICE_SCOPES as readonly string[]).includes(s),
    ) ||
    (org !== undefined && (typeof org !== 'string' || !UUID.test(org))) ||
    (repo !== undefined && (typeof repo !== 'string' || !UUID.test(repo)))
  ) {
    throw new ServiceAuthError('invalid_claims');
  }
  if (exp - iat > MAX_TOKEN_LIFETIME_SECONDS) throw new ServiceAuthError('lifetime_too_long');
  const nowSeconds = opts.now ?? Math.floor(Date.now() / 1000);
  if (iat > nowSeconds + CLOCK_SKEW_SECONDS) throw new ServiceAuthError('not_yet_valid');
  return {
    iss: iss as ServiceIssuer,
    aud: opts.audience,
    sub,
    scope: scope as ServiceScope[],
    org: org as string | undefined,
    repo: repo as string | undefined,
    iat,
    exp,
    jti,
  };
}

function classify(err: unknown): ServiceAuthFailureReason {
  if (err instanceof joseErrors.JWTExpired) return 'expired';
  if (err instanceof joseErrors.JWSSignatureVerificationFailed) return 'bad_signature';
  if (err instanceof joseErrors.JWTClaimValidationFailed) {
    if (err.claim === 'aud') return 'wrong_audience';
    if (err.claim === 'iat' || err.claim === 'nbf') return 'not_yet_valid';
    return 'invalid_claims';
  }
  if (err instanceof joseErrors.JWSInvalid || err instanceof joseErrors.JWTInvalid) {
    return 'malformed';
  }
  return 'malformed';
}

/** One-time `jti` claim; resolves false when the id was already seen. */
export interface JtiStore {
  claim(jti: string, ttlSeconds: number): Promise<boolean>;
}

/** Minimal Redis surface used by the stores in this module. */
export interface RedisLike {
  set(key: string, value: string, ex: 'EX', seconds: number, nx: 'NX'): Promise<'OK' | null>;
}

/** API-side replay cache: `SET rg:jti:{jti} NX EX 120`. */
export class RedisJtiStore implements JtiStore {
  constructor(private readonly redis: RedisLike) {}

  async claim(jti: string, ttlSeconds: number): Promise<boolean> {
    return (await this.redis.set(`rg:jti:${jti}`, '1', 'EX', ttlSeconds, 'NX')) === 'OK';
  }
}

/** In-memory LRU replay cache (engine-style; used in tests and as a Redis-outage fallback). */
export class MemoryJtiStore implements JtiStore {
  private readonly seen = new Map<string, number>();

  constructor(
    private readonly capacity = 10_000,
    private readonly clock: () => number = Date.now,
  ) {}

  claim(jti: string, ttlSeconds: number): Promise<boolean> {
    const now = this.clock();
    const expiry = this.seen.get(jti);
    if (expiry !== undefined && expiry > now) return Promise.resolve(false);
    this.seen.delete(jti);
    this.seen.set(jti, now + ttlSeconds * 1000);
    while (this.seen.size > this.capacity) {
      const oldest = this.seen.keys().next().value as string;
      this.seen.delete(oldest);
    }
    return Promise.resolve(true);
  }
}
