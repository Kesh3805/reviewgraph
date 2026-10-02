import { createHmac } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import {
  MemoryJtiStore,
  ServiceAuthError,
  ServiceKeyRing,
  signServiceToken,
  verifyServiceToken,
  type ServiceTokenClaims,
} from '../../src/internal/service-token';

const FIXTURE_DIR = resolve(__dirname, '../../../../packages/contracts/fixtures/service-token');
const NOW = 1_700_000_000;
const ORG = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2b';
const REPO = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2c';
const SECRET = Buffer.from('golden-vector-secret-0123456789ab', 'utf8');
const RING = ServiceKeyRing.from([{ kid: 'k1', secret: SECRET }]);
const GOLDEN_CLAIMS: ServiceTokenClaims = {
  iss: 'rg-worker',
  aud: 'rg-api',
  sub: 'worker-1',
  scope: ['clone-credentials'],
  org: ORG,
  repo: REPO,
  iat: NOW,
  exp: NOW + 60,
  jti: 'golden-jti-0001',
};

const b64url = (v: Buffer | string): string => Buffer.from(v).toString('base64url');

/**
 * Builds a token the way the Rust side does (`jsonwebtoken`): serde field order, header
 * `{"typ":"JWT","alg":"HS256","kid":...}`. Used to pin the wire format from the other side.
 */
function rustStyleToken(claims: ServiceTokenClaims, kid: string, secret: Buffer): string {
  const header = b64url(JSON.stringify({ typ: 'JWT', alg: 'HS256', kid }));
  const payload = b64url(
    JSON.stringify({
      iss: claims.iss,
      aud: claims.aud,
      sub: claims.sub,
      scope: claims.scope,
      org: claims.org,
      repo: claims.repo,
      iat: claims.iat,
      exp: claims.exp,
      jti: claims.jti,
    }),
  );
  const sig = createHmac('sha256', secret).update(`${header}.${payload}`).digest('base64url');
  return `${header}.${payload}.${sig}`;
}

function golden(name: string, build: () => unknown): { token: string } & Record<string, unknown> {
  const path = resolve(FIXTURE_DIR, name);
  if (!existsSync(path) && process.env.UPDATE_GOLDEN) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, `${JSON.stringify(build(), null, 2)}\n`);
  }
  return JSON.parse(readFileSync(path, 'utf8')) as { token: string } & Record<string, unknown>;
}

const sign = (overrides: Partial<Parameters<typeof signServiceToken>[1]> = {}, ring = RING) =>
  signServiceToken(ring, {
    iss: 'rg-worker',
    aud: 'rg-api',
    sub: 'worker-1',
    scope: ['clone-credentials'],
    now: NOW,
    jti: 'jti-1',
    ...overrides,
  });

async function reasonOf(promise: Promise<unknown>): Promise<string | undefined> {
  try {
    await promise;
    return undefined;
  } catch (err) {
    return err instanceof ServiceAuthError ? err.reason : `unexpected:${String(err)}`;
  }
}

describe('service token', () => {
  it('ts_signs_rust_verifies: the TS token matches the golden vector byte for byte', async () => {
    const token = await signServiceToken(RING, {
      iss: GOLDEN_CLAIMS.iss,
      aud: GOLDEN_CLAIMS.aud,
      sub: GOLDEN_CLAIMS.sub,
      scope: GOLDEN_CLAIMS.scope,
      org: ORG,
      repo: REPO,
      now: NOW,
      jti: GOLDEN_CLAIMS.jti,
    });
    const stored = golden('ts-signed.json', () => ({
      description:
        'Signed by apps/api (TS, jose). The Rust verifier must accept it at now=1700000000.',
      kid: 'k1',
      secret_utf8: SECRET.toString('utf8'),
      now: NOW,
      audience: 'rg-api',
      claims: GOLDEN_CLAIMS,
      token,
    }));
    expect(token).toBe(stored.token);
    await expect(
      verifyServiceToken(RING, token, { audience: 'rg-api', now: NOW }),
    ).resolves.toEqual(GOLDEN_CLAIMS);
  });

  it('rust_signs_ts_verifies: a Rust-style token is accepted', async () => {
    const vector = golden('rust-signed.json', () => ({
      description:
        'Built with the jsonwebtoken field layout (serde order). Regenerate from engine/crates/pipeline once the Rust signer lands; the TS verifier must keep accepting it.',
      kid: 'k1',
      secret_utf8: SECRET.toString('utf8'),
      now: NOW,
      audience: 'rg-api',
      claims: GOLDEN_CLAIMS,
      token: rustStyleToken(GOLDEN_CLAIMS, 'k1', SECRET),
    }));
    expect(vector.token).toBe(rustStyleToken(GOLDEN_CLAIMS, 'k1', SECRET));
    await expect(
      verifyServiceToken(RING, vector.token, { audience: 'rg-api', now: NOW }),
    ).resolves.toEqual(GOLDEN_CLAIMS);
  });

  it('expired_token_401: rejects past exp plus skew, accepts within skew', async () => {
    const token = await sign();
    expect(
      await reasonOf(verifyServiceToken(RING, token, { audience: 'rg-api', now: NOW + 91 })),
    ).toBe('expired');
    await expect(
      verifyServiceToken(RING, token, { audience: 'rg-api', now: NOW + 89 }),
    ).resolves.toBeDefined();
  });

  it('wrong_audience_401', async () => {
    const token = await sign({ aud: 'rg-engine' });
    expect(await reasonOf(verifyServiceToken(RING, token, { audience: 'rg-api', now: NOW }))).toBe(
      'wrong_audience',
    );
  });

  it('rotation_old_kid_still_verifies: first key signs, every key verifies', async () => {
    const oldRing = ServiceKeyRing.from([{ kid: 'old', secret: Buffer.alloc(32, 7) }]);
    const newRing = ServiceKeyRing.parse(
      `new:${Buffer.alloc(32, 9).toString('base64')},old:${Buffer.alloc(32, 7).toString('base64')}`,
    );
    const oldToken = await sign({}, oldRing);
    await expect(
      verifyServiceToken(newRing, oldToken, { audience: 'rg-api', now: NOW }),
    ).resolves.toBeDefined();
    const newToken = await sign({}, newRing);
    expect(newRing.signing.kid).toBe('new');
    // A verifier that has dropped the new key rejects it.
    expect(
      await reasonOf(verifyServiceToken(oldRing, newToken, { audience: 'rg-api', now: NOW })),
    ).toBe('unknown_kid');
  });

  it('rejects a token signed with the wrong secret, none-alg and garbage', async () => {
    const other = ServiceKeyRing.from([{ kid: 'k1', secret: Buffer.alloc(32, 3) }]);
    const forged = await sign({}, other);
    expect(await reasonOf(verifyServiceToken(RING, forged, { audience: 'rg-api', now: NOW }))).toBe(
      'bad_signature',
    );
    const none = `${b64url(JSON.stringify({ alg: 'none', kid: 'k1' }))}.${b64url('{}')}.`;
    expect(await reasonOf(verifyServiceToken(RING, none, { audience: 'rg-api', now: NOW }))).toBe(
      'malformed',
    );
    expect(await reasonOf(verifyServiceToken(RING, 'not-a-jwt', { audience: 'rg-api' }))).toBe(
      'malformed',
    );
  });

  it('rejects a lifetime above 60 s even when correctly signed', async () => {
    const token = rustStyleToken({ ...GOLDEN_CLAIMS, exp: NOW + 61 }, 'k1', SECRET);
    expect(await reasonOf(verifyServiceToken(RING, token, { audience: 'rg-api', now: NOW }))).toBe(
      'lifetime_too_long',
    );
    await expect(sign({ ttlSeconds: 61 })).rejects.toThrow('lifetime');
  });

  it('rejects unknown scopes and non-uuid org/repo claims', async () => {
    const bad = rustStyleToken({ ...GOLDEN_CLAIMS, scope: ['everything' as never] }, 'k1', SECRET);
    expect(await reasonOf(verifyServiceToken(RING, bad, { audience: 'rg-api', now: NOW }))).toBe(
      'invalid_claims',
    );
    const badRepo = rustStyleToken({ ...GOLDEN_CLAIMS, repo: '../etc' }, 'k1', SECRET);
    expect(
      await reasonOf(verifyServiceToken(RING, badRepo, { audience: 'rg-api', now: NOW })),
    ).toBe('invalid_claims');
  });

  it('key ring parsing never echoes key material', () => {
    const secretish = Buffer.from('short').toString('base64');
    expect(() => ServiceKeyRing.parse(`k1:${secretish}`)).toThrow(/at least 32 bytes/);
    try {
      ServiceKeyRing.parse(`k1:${secretish}`);
    } catch (err) {
      expect(String(err)).not.toContain(secretish);
    }
    expect(() => ServiceKeyRing.parse('nokid')).toThrow();
  });

  it('memory jti store rejects replays within ttl and evicts beyond capacity', async () => {
    let now = 0;
    const store = new MemoryJtiStore(2, () => now);
    expect(await store.claim('a', 120)).toBe(true);
    expect(await store.claim('a', 120)).toBe(false);
    now = 121_000;
    expect(await store.claim('a', 120)).toBe(true);
    await store.claim('b', 120);
    await store.claim('c', 120);
    expect(await store.claim('a', 120)).toBe(true);
  });
});
