import { Controller, Get, Param, Post } from '@nestjs/common';
import type { NestExpressApplication } from '@nestjs/platform-express';
import RedisMock from 'ioredis-mock';
import request from 'supertest';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import { ServiceAuth, ServiceCaller } from '../../src/internal/service-auth.guard';
import {
  ServiceKeyRing,
  signServiceToken,
  type ServiceTokenClaims,
} from '../../src/internal/service-token';
import { VALID_ENV, createTestApp } from '../helpers';

const REPO = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2c';
const OTHER_REPO = '0197a1c2-3d4e-7f50-8a6b-7c8d9e0f1a2d';
const SECRET = Buffer.from(VALID_ENV.SERVICE_JWT_SECRET as string, 'utf8');
const RING = ServiceKeyRing.from([{ kid: 'default', secret: SECRET }]);

@Controller('internal')
class FakeInternalController {
  @Post('repositories/:id/clone-credentials')
  @ServiceAuth({ scopes: ['clone-credentials'], repoParam: 'id' })
  clone(@Param('id') id: string, @ServiceCaller() caller: ServiceTokenClaims | undefined) {
    return { repo: id, caller: caller?.sub };
  }

  @Get('graph/:id')
  @ServiceAuth({ scopes: ['graph:read'] })
  graph(@Param('id') id: string) {
    return { id };
  }

  @Get('forgotten-policy')
  forgotten() {
    return { leaked: true };
  }
}

function token(overrides: Partial<Parameters<typeof signServiceToken>[1]> = {}): Promise<string> {
  return signServiceToken(RING, {
    iss: 'rg-worker',
    aud: 'rg-api',
    sub: 'worker-1',
    scope: ['clone-credentials'],
    repo: REPO,
    ...overrides,
  });
}

describe('service auth guard (e2e)', () => {
  let app: NestExpressApplication;

  beforeEach(async () => {
    resetCounterTotals();
    app = await createTestApp(undefined, [FakeInternalController]);
    await app.init();
  });
  afterEach(async () => {
    await app.close();
  });

  const clone = (id = REPO) =>
    request(app.getHttpServer()).post(`/internal/repositories/${id}/clone-credentials`);

  it('accepts a valid token and exposes the caller claims', async () => {
    const res = await clone().set('Authorization', `Bearer ${await token()}`);
    expect(res.status).toBe(201);
    expect(res.body).toEqual({ repo: REPO, caller: 'worker-1' });
  });

  it('expired_token_401 with an empty problem body', async () => {
    const now = Math.floor(Date.now() / 1000) - 600;
    const res = await clone().set('Authorization', `Bearer ${await token({ now })}`);
    expect(res.status).toBe(401);
    expect(res.body.detail).toBeUndefined();
    expect(JSON.stringify(res.body)).not.toMatch(/expired|signature|token/i);
    expect(
      counterTotal('service_auth_failures_total', {
        reason: 'expired',
        route: '/internal/repositories/:id/clone-credentials',
      }),
    ).toBe(1);
  });

  it('wrong_audience_401', async () => {
    const res = await clone().set('Authorization', `Bearer ${await token({ aud: 'rg-engine' })}`);
    expect(res.status).toBe(401);
  });

  it('replayed_jti_401', async () => {
    const t = await token();
    expect((await clone().set('Authorization', `Bearer ${t}`)).status).toBe(201);
    expect((await clone().set('Authorization', `Bearer ${t}`)).status).toBe(401);
    expect(
      counterTotal('service_auth_failures_total', {
        reason: 'replayed',
        route: '/internal/repositories/:id/clone-credentials',
      }),
    ).toBe(1);
  });

  it('repo_claim_mismatch_403', async () => {
    const res = await clone(OTHER_REPO).set('Authorization', `Bearer ${await token()}`);
    expect(res.status).toBe(403);
  });

  it('403 when the scope is missing', async () => {
    const res = await request(app.getHttpServer())
      .get('/internal/graph/abc')
      .set('Authorization', `Bearer ${await token()}`);
    expect(res.status).toBe(403);
  });

  it('fails closed when the replay store is unavailable', async () => {
    await app.close();
    const broken = new RedisMock();
    broken.set = () => Promise.reject(new Error('down')) as never;
    app = await createTestApp(undefined, [FakeInternalController], { redis: broken });
    await app.init();
    const res = await clone().set('Authorization', `Bearer ${await token()}`);
    expect(res.status).toBe(503);
  });

  it('every /internal route rejects unauthenticated calls (route table)', async () => {
    const server = app.getHttpAdapter().getInstance() as {
      router?: { stack: { route?: { path: string; methods: Record<string, boolean> } }[] };
      _router?: { stack: { route?: { path: string; methods: Record<string, boolean> } }[] };
    };
    const stack = (server.router ?? server._router)?.stack ?? [];
    const routes = stack
      .filter((layer) => layer.route?.path.startsWith('/internal'))
      .flatMap((layer) =>
        Object.keys(layer.route!.methods).map((method) => ({ method, path: layer.route!.path })),
      );
    expect(routes.length).toBeGreaterThanOrEqual(3);
    for (const { method, path } of routes) {
      const url = path.replace(/:\w+/g, REPO);
      const agent = request(app.getHttpServer()) as unknown as Record<
        string,
        (u: string) => request.Test
      >;
      const call = agent[method];
      if (!call) throw new Error('unsupported method ' + method);
      const res = await call.call(agent, url);
      expect([method, path, res.status]).toEqual([method, path, 401]);
    }
  });

  it('an internal route without a policy is denied even with a valid token', async () => {
    const res = await request(app.getHttpServer())
      .get('/internal/forgotten-policy')
      .set('Authorization', `Bearer ${await token()}`);
    expect(res.status).toBe(403);
  });

  it('public routes are unaffected', async () => {
    const res = await request(app.getHttpServer()).get('/api/v1/health/live');
    expect(res.status).toBe(200);
  });
});
