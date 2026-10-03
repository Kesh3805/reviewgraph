import { createHash } from 'node:crypto';
import type { NestExpressApplication } from '@nestjs/platform-express';
import RedisMock from 'ioredis-mock';
import request from 'supertest';
import { SessionService } from '../../src/auth/session.service';
import { signWebhookBody } from '../../src/webhooks/signature';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import { createTestApp } from '../helpers';
import { createAuthApp, WEB_ORIGIN } from '../helpers/auth-app';
import { FakeOAuth } from '../helpers/fake-oauth';

const LOGIN = '/api/v1/auth/github/login';
const CALLBACK = '/api/v1/auth/github/callback';
const PROBE = '/api/v1/csrf-probe';
const USER = { userId: '0190f3a2-0000-7000-8000-0000000000aa', sessionId: 's-1' };

/** Sessions live in memory here; the database-backed behavior is covered by integration tests. */
class FakeSessions {
  live = new Set(['valid-session']);
  verify = (token: string) => Promise.resolve(this.live.has(token) ? USER : null);
}

describe('auth (unit, no database)', () => {
  const oauth = new FakeOAuth();
  const redis = new RedisMock();
  const sessions = new FakeSessions();
  let app: NestExpressApplication;

  beforeAll(async () => {
    await oauth.start();
    app = await createAuthApp(oauth, {
      redis,
      configure: (builder) => builder.overrideProvider(SessionService).useValue(sessions),
    });
    await app.init();
  });
  afterAll(async () => {
    await app.close();
    await oauth.stop();
  });
  beforeEach(() => {
    resetCounterTotals();
    sessions.live = new Set(['valid-session']);
    oauth.apiFailure = undefined;
  });

  /** Starts a login and returns the state, the PKCE challenge and the state cookie. */
  async function startLogin() {
    const res = await request(app.getHttpServer()).get(LOGIN).expect(302);
    const location = new URL(res.headers.location as string);
    const cookie = (res.headers['set-cookie'] as unknown as string[]).find((c) =>
      c.startsWith('rg_oauth_state='),
    )!;
    return {
      location,
      state: location.searchParams.get('state')!,
      challenge: location.searchParams.get('code_challenge')!,
      cookie,
      cookiePair: cookie.split(';')[0]!,
    };
  }

  describe('login redirect', () => {
    it('uses a random single-use state, PKCE S256 and a signed HttpOnly cookie', async () => {
      const { location, state, challenge, cookie } = await startLogin();
      expect(location.origin).toBe(oauth.url);
      expect(location.pathname).toBe('/login/oauth/authorize');
      expect(location.searchParams.get('client_id')).toBe('client-id');
      expect(location.searchParams.get('code_challenge_method')).toBe('S256');
      expect(Buffer.from(state, 'base64url')).toHaveLength(32);
      const verifier = await redis.get(`rg:oauth:state:${state}`);
      expect(verifier).toBeTruthy();
      expect(createHash('sha256').update(verifier!).digest('base64url')).toBe(challenge);
      expect(cookie).toMatch(/HttpOnly/i);
      expect(cookie).toMatch(/SameSite=Lax/i);
      expect(cookie).toMatch(/Max-Age=600/i);
      expect(cookie).toMatch(/Path=\/api\/v1\/auth\/github/);
    });

    it('is not served when GitHub is disabled', async () => {
      const disabled = await createTestApp(undefined, [], { redis });
      await disabled.init();
      await request(disabled.getHttpServer()).get(LOGIN).expect(404);
      await disabled.close();
    });
  });

  describe('callback', () => {
    it('oauth_state_mismatch_rejected', async () => {
      const { cookiePair } = await startLogin();
      await request(app.getHttpServer())
        .get(`${CALLBACK}?code=abc&state=${'x'.repeat(43)}`)
        .set('cookie', cookiePair)
        .expect(400);
      await request(app.getHttpServer()).get(`${CALLBACK}?code=abc&state=nope`).expect(400);
      await request(app.getHttpServer())
        .get(`${CALLBACK}?code=abc`)
        .set('cookie', cookiePair)
        .expect(400);
      expect(counterTotal('auth_logins_total', { result: 'state_invalid' })).toBe(3);
      expect(oauth.tokenRequests).toHaveLength(0);
    });

    it('rejects a state cookie that was not signed by this server', async () => {
      const { state } = await startLogin();
      const forged = `rg_oauth_state=${Buffer.from(JSON.stringify({ st: state })).toString('base64url')}`;
      await request(app.getHttpServer())
        .get(`${CALLBACK}?code=abc&state=${state}`)
        .set('cookie', forged)
        .expect(400);
    });

    it('oauth_state_single_use', async () => {
      const { state, cookiePair } = await startLogin();
      const first = await request(app.getHttpServer())
        .get(`${CALLBACK}?code=unknown&state=${state}`)
        .set('cookie', cookiePair)
        .expect(400);
      expect(first.body.detail).toMatch(/code/);
      expect(counterTotal('auth_logins_total', { result: 'invalid_code' })).toBe(1);
      // The same state and cookie again: the state is gone, so GitHub is never asked.
      const tokenRequests = oauth.tokenRequests.length;
      const replay = await request(app.getHttpServer())
        .get(`${CALLBACK}?code=unknown&state=${state}`)
        .set('cookie', cookiePair)
        .expect(400);
      expect(replay.body.detail).toMatch(/state/);
      expect(counterTotal('auth_logins_total', { result: 'state_invalid' })).toBe(1);
      expect(oauth.tokenRequests).toHaveLength(tokenRequests);
      expect(await redis.get(`rg:oauth:state:${state}`)).toBeNull();
    });

    it('sends the PKCE verifier to the token endpoint', async () => {
      const { state, cookiePair, challenge } = await startLogin();
      const verifierBefore = await redis.get(`rg:oauth:state:${state}`);
      await request(app.getHttpServer())
        .get(`${CALLBACK}?code=unknown&state=${state}`)
        .set('cookie', cookiePair)
        .expect(400);
      const sent = oauth.tokenRequests.at(-1)!;
      expect(sent.code_verifier).toBe(verifierBefore);
      expect(createHash('sha256').update(sent.code_verifier!).digest('base64url')).toBe(challenge);
      expect(sent.client_secret).toBe('client-secret');
    });

    it('redirects to the login page when GitHub is unavailable', async () => {
      const { state, cookiePair } = await startLogin();
      oauth.apiFailure = 503;
      const res = await request(app.getHttpServer())
        .get(`${CALLBACK}?code=${oauth.newCode()}&state=${state}`)
        .set('cookie', cookiePair)
        .expect(302);
      expect(res.headers.location).toBe(`${WEB_ORIGIN}/login?error=github_unavailable`);
      expect(counterTotal('auth_logins_total', { result: 'github_unavailable' })).toBe(1);
      expect(String(res.headers['set-cookie'] ?? '')).not.toContain('rg_session=');
    });

    it('redirects to the login page when the user denies access', async () => {
      const { state, cookiePair } = await startLogin();
      const res = await request(app.getHttpServer())
        .get(`${CALLBACK}?error=access_denied&state=${state}`)
        .set('cookie', cookiePair)
        .expect(302);
      expect(res.headers.location).toBe(`${WEB_ORIGIN}/login?error=access_denied`);
    });
  });

  describe('session and CSRF guards', () => {
    const authed = (method: 'post' | 'patch' | 'delete' | 'get') => {
      const agent = request(app.getHttpServer());
      const req = { post: agent.post, patch: agent.patch, delete: agent.delete, get: agent.get }[
        method
      ];
      return req
        .call(agent, PROBE)
        .set('cookie', 'rg_session=valid-session; rg_csrf=csrf-token-123');
    };

    it('requires a session on every non-public route', async () => {
      await request(app.getHttpServer()).get(PROBE).expect(401);
      await request(app.getHttpServer()).get('/api/v1/auth/me').expect(401);
      await authed('get').expect(200);
    });

    it('keeps health, webhooks and OAuth login public', async () => {
      await request(app.getHttpServer()).get('/api/v1/health/live').expect(200);
      const body = Buffer.from('{}');
      await request(app.getHttpServer())
        .post('/api/v1/webhooks/github')
        .set('content-type', 'application/json')
        .set('x-github-event', 'ping')
        .set('x-github-delivery', 'ping-1')
        .set('x-hub-signature-256', signWebhookBody('whsec_auth_tests_0123456789', body))
        .send(body.toString())
        .expect(202);
      await request(app.getHttpServer()).get(LOGIN).expect(302);
    });

    it('revoked_session_401', async () => {
      await authed('get').expect(200);
      sessions.live.delete('valid-session');
      await authed('get').expect(401);
    });

    it('mutation_without_csrf_header_403', async () => {
      await authed('post').set('origin', WEB_ORIGIN).expect(403);
      expect(counterTotal('csrf_rejections_total', { reason: 'token' })).toBe(1);
    });

    it('rejects a csrf header that does not match the cookie', async () => {
      await authed('post').set('origin', WEB_ORIGIN).set('x-csrf-token', 'other').expect(403);
    });

    it('mutation_with_foreign_origin_403', async () => {
      await authed('post')
        .set('origin', 'https://evil.example')
        .set('x-csrf-token', 'csrf-token-123')
        .expect(403);
      expect(counterTotal('csrf_rejections_total', { reason: 'origin' })).toBe(1);
    });

    it('requires an Origin header on mutations', async () => {
      await authed('post').set('x-csrf-token', 'csrf-token-123').expect(403);
    });

    it('accepts a mutation with the matching token and the web origin', async () => {
      for (const method of ['post', 'patch', 'delete'] as const) {
        await authed(method)
          .set('origin', WEB_ORIGIN)
          .set('x-csrf-token', 'csrf-token-123')
          .expect((r) => expect([200, 201]).toContain(r.status));
      }
    });

    it('legacy_csrf_regression_cross_site_form_post_rejected', async () => {
      // What a malicious page can do: a form POST that carries the victim cookies (SameSite=Lax
      // already withholds them on cross-site POSTs; this proves the server refuses it anyway).
      for (const method of ['post', 'patch', 'delete'] as const) {
        const res = await authed(method)
          .set('origin', 'https://evil.example')
          .type('form')
          .send({ anything: '1' })
          .expect(403);
        expect(res.headers['content-type']).toContain('application/problem+json');
      }
      // No Origin and no token at all, e.g. a bare form post from a sandboxed frame.
      await authed('post').type('form').send({ a: '1' }).expect(403);
    });

    it('answers 401 before CSRF or tenancy for an unauthenticated mutation', async () => {
      await request(app.getHttpServer()).post(`${PROBE}/tenant`).expect(401);
      await request(app.getHttpServer()).post(PROBE).expect(401);
    });
  });
});
