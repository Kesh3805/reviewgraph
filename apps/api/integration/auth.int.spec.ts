import type { NestExpressApplication } from '@nestjs/platform-express';
import { Redis } from 'ioredis';
import { sql } from 'kysely';
import request from 'supertest';
import { sessionCacheKey } from '../src/auth/session.service';
import { createAuthApp, WEB_ORIGIN } from '../test/helpers/auth-app';
import { FAKE_USER_TOKEN, FakeOAuth } from '../test/helpers/fake-oauth';
import { adminDb, cleanup, seedOrg, type SeededOrg } from './seed';

const LOGIN = '/api/v1/auth/github/login';
const CALLBACK = '/api/v1/auth/github/callback';

type Jar = Record<string, string>;

function jarFrom(setCookie: string[] | string | undefined, jar: Jar = {}): Jar {
  for (const line of [setCookie ?? []].flat()) {
    const [pair] = line.split(';');
    const eq = pair!.indexOf('=');
    jar[pair!.slice(0, eq)] = decodeURIComponent(pair!.slice(eq + 1));
  }
  return jar;
}

const header = (jar: Jar): string =>
  Object.entries(jar)
    .map(([k, v]) => `${k}=${v}`)
    .join('; ');

describe('auth (integration)', () => {
  const admin = adminDb();
  const oauth = new FakeOAuth();
  const redis = new Redis(process.env.RG_TEST_REDIS_URL!);
  let app: NestExpressApplication;
  const orgs: SeededOrg[] = [];
  const userIds: string[] = [];
  let teamOrg: SeededOrg;
  let personalOrg: SeededOrg;
  let otherOrg: SeededOrg;

  beforeAll(async () => {
    await oauth.start();
    app = await createAuthApp(oauth, {
      redis,
      env: { DATABASE_URL: process.env.RG_TEST_DATABASE_URL!, DB_APP_ROLE: 'rg_api' },
    });
    await app.init();
    teamOrg = await seedOrg(admin);
    personalOrg = await seedOrg(admin);
    otherOrg = await seedOrg(admin);
    orgs.push(teamOrg, personalOrg, otherOrg);
  });

  afterAll(async () => {
    await app.close();
    await oauth.stop();
    await cleanup(
      admin,
      orgs.map((o) => o.organizationId),
      userIds,
    );
    await admin.destroy();
    redis.disconnect();
  });

  beforeEach(async () => {
    // A fresh GitHub identity per test so users do not collide with each other.
    oauth.user = {
      id: Math.floor(Math.random() * 1e9) + 1,
      login: `octo-${Math.random().toString(36).slice(2, 8)}`,
      name: 'Octo User',
      email: null,
      avatar_url: 'https://avatars.example/u/1',
    };
    await admin
      .updateTable('provider_installations')
      .set({ account_type: 'organization', account_login: 'team' })
      .where('id', '=', personalOrg.installationId)
      .execute();
    oauth.installations = [];
  });

  /** Runs the whole flow against the fake GitHub and returns the resulting cookies. */
  async function login(): Promise<{ jar: Jar; location: string; setCookie: string[] }> {
    const start = await request(app.getHttpServer()).get(LOGIN).expect(302);
    const state = new URL(start.headers.location as string).searchParams.get('state')!;
    const jar = jarFrom(start.headers['set-cookie']);
    const res = await request(app.getHttpServer())
      .get(`${CALLBACK}?code=${oauth.newCode()}&state=${state}`)
      .set('cookie', header(jar))
      .expect(302);
    const setCookie = [res.headers['set-cookie'] ?? []].flat() as string[];
    return { jar: jarFrom(setCookie), location: res.headers.location as string, setCookie };
  }

  const roles = async (userId: string) =>
    Object.fromEntries(
      (
        await admin
          .selectFrom('memberships')
          .select(['organization_id', 'role'])
          .where('user_id', '=', userId)
          .execute()
      ).map((m) => [m.organization_id, m.role]),
    );

  const userByLogin = async (login: string) => {
    const user = await admin
      .selectFrom('users')
      .selectAll()
      .where('login', '=', login)
      .executeTakeFirstOrThrow();
    userIds.push(user.id);
    return user;
  };

  it('callback_creates_session_and_memberships', async () => {
    // personalOrg becomes the user personal account; the unknown installation is ignored.
    await admin
      .updateTable('provider_installations')
      .set({ account_type: 'user', account_login: oauth.user.login })
      .where('id', '=', personalOrg.installationId)
      .execute();
    oauth.installations = [
      { id: teamOrg.providerInstallationId, account: { login: 'team', type: 'Organization' } },
      {
        id: personalOrg.providerInstallationId,
        account: { login: oauth.user.login, type: 'User' },
      },
      { id: 999_999_999, account: { login: 'stranger', type: 'Organization' } },
    ];

    const { jar, location, setCookie } = await login();
    expect(location).toBe(`${WEB_ORIGIN}/`);

    const session = setCookie.find((c) => c.startsWith('rg_session='))!;
    expect(session).toMatch(/HttpOnly/i);
    expect(session).toMatch(/SameSite=Lax/i);
    expect(session).toMatch(/Path=\//);
    expect(session).toMatch(/Max-Age=43200/);
    const csrf = setCookie.find((c) => c.startsWith('rg_csrf='))!;
    expect(csrf).not.toMatch(/HttpOnly/i);
    expect(csrf).toMatch(/SameSite=Lax/i);

    const user = await userByLogin(oauth.user.login);
    expect(user.provider_user_id).toBe(String(oauth.user.id));
    expect(user.avatar_url).toBe('https://avatars.example/u/1');
    expect(await roles(user.id)).toEqual({
      [teamOrg.organizationId]: 'member',
      [personalOrg.organizationId]: 'owner',
    });

    const sessions = await admin
      .selectFrom('sessions')
      .selectAll()
      .where('user_id', '=', user.id)
      .execute();
    expect(sessions).toHaveLength(1);
    expect(sessions[0]!.revoked_at).toBeNull();
    expect(sessions[0]!.expires_at.getTime() - Date.now()).toBeGreaterThan(11.9 * 3600_000);

    const me = await request(app.getHttpServer())
      .get('/api/v1/auth/me')
      .set('cookie', header(jar))
      .expect(200);
    expect(me.body.user.login).toBe(oauth.user.login);
    expect(me.body.organizations.map((o: { id: string }) => o.id).sort()).toEqual(
      [teamOrg.organizationId, personalOrg.organizationId].sort(),
    );
  });

  it('refreshes memberships on every login (access revoked on GitHub is revoked here)', async () => {
    oauth.installations = [
      { id: teamOrg.providerInstallationId, account: { login: 'team', type: 'Organization' } },
      { id: otherOrg.providerInstallationId, account: { login: 'other', type: 'Organization' } },
    ];
    await login();
    const user = await userByLogin(oauth.user.login);
    expect(Object.keys(await roles(user.id)).sort()).toEqual(
      [teamOrg.organizationId, otherOrg.organizationId].sort(),
    );

    // A dashboard-assigned admin role is kept; the lost installation disappears.
    await admin
      .updateTable('memberships')
      .set({ role: 'admin' })
      .where('user_id', '=', user.id)
      .where('organization_id', '=', teamOrg.organizationId)
      .execute();
    oauth.installations = [
      { id: teamOrg.providerInstallationId, account: { login: 'team', type: 'Organization' } },
    ];
    await login();
    expect(await roles(user.id)).toEqual({ [teamOrg.organizationId]: 'admin' });
  });

  it('oauth_token_not_persisted', async () => {
    oauth.installations = [
      { id: teamOrg.providerInstallationId, account: { login: 'team', type: 'Organization' } },
    ];
    const { setCookie } = await login();
    expect(oauth.tokenRequests.length).toBeGreaterThan(0);
    const user = await userByLogin(oauth.user.login);
    for (const table of [
      'users',
      'sessions',
      'memberships',
      'organizations',
      'provider_installations',
    ]) {
      const { rows } = await sql<{ dump: string }>`
        select coalesce(string_agg(to_jsonb(t)::text, ' '), '') as dump from ${sql.table(table)} t`.execute(
        admin,
      );
      expect(rows[0]!.dump).not.toContain(FAKE_USER_TOKEN);
    }
    expect(user.login).toBe(oauth.user.login);
    // Nor does the browser ever receive it.
    expect(setCookie.join(';')).not.toContain(FAKE_USER_TOKEN);
    const keys = await redis.keys('rg:*');
    for (const key of keys) {
      expect(String((await redis.get(key).catch(() => '')) ?? '')).not.toContain(FAKE_USER_TOKEN);
    }
  });

  it('revoked_session_401 after logout', async () => {
    const { jar } = await login();
    const cookie = header(jar);
    await request(app.getHttpServer()).get('/api/v1/auth/me').set('cookie', cookie).expect(200);
    await request(app.getHttpServer())
      .post('/api/v1/auth/logout')
      .set('cookie', cookie)
      .set('origin', WEB_ORIGIN)
      .set('x-csrf-token', jar.rg_csrf!)
      .expect(204);
    await request(app.getHttpServer()).get('/api/v1/auth/me').set('cookie', cookie).expect(401);
    const user = await userByLogin(oauth.user.login);
    const row = await admin
      .selectFrom('sessions')
      .select('revoked_at')
      .where('user_id', '=', user.id)
      .executeTakeFirstOrThrow();
    expect(row.revoked_at).not.toBeNull();
  });

  it('logout without the CSRF token is refused and keeps the session', async () => {
    const { jar } = await login();
    const cookie = header(jar);
    await request(app.getHttpServer())
      .post('/api/v1/auth/logout')
      .set('cookie', cookie)
      .set('origin', 'https://evil.example')
      .expect(403);
    await request(app.getHttpServer()).get('/api/v1/auth/me').set('cookie', cookie).expect(200);
  });

  it('a session revoked in the database stops working once the cache entry is gone', async () => {
    const { jar } = await login();
    const cookie = header(jar);
    await request(app.getHttpServer()).get('/api/v1/auth/me').set('cookie', cookie).expect(200);
    const user = await userByLogin(oauth.user.login);
    const row = await admin
      .updateTable('sessions')
      .set({ revoked_at: sql<Date>`now()` })
      .where('user_id', '=', user.id)
      .returning('id')
      .executeTakeFirstOrThrow();
    // Within the 60 s cache window the entry is still trusted; expiring it ends that.
    await redis.del(sessionCacheKey(row.id));
    await request(app.getHttpServer()).get('/api/v1/auth/me').set('cookie', cookie).expect(401);
  });

  it('an expired session is rejected', async () => {
    const { jar } = await login();
    const cookie = header(jar);
    const user = await userByLogin(oauth.user.login);
    const row = await admin
      .updateTable('sessions')
      .set({
        expires_at: sql<Date>`now() + interval '1 second'`,
        created_at: sql<Date>`now() - interval '1 hour'`,
      })
      .where('user_id', '=', user.id)
      .returning('id')
      .executeTakeFirstOrThrow();
    await redis.del(sessionCacheKey(row.id));
    await new Promise((resolve) => setTimeout(resolve, 1200));
    await request(app.getHttpServer()).get('/api/v1/auth/me').set('cookie', cookie).expect(401);
  });

  it('rejects a session JWT signed with another key', async () => {
    const { jar } = await login();
    const [head, body] = jar.rg_session!.split('.');
    const forged = `${head}.${body}.${'A'.repeat(43)}`;
    await request(app.getHttpServer())
      .get('/api/v1/auth/me')
      .set('cookie', `rg_session=${forged}`)
      .expect(401);
  });
});
