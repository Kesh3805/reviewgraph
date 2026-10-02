import type { NestExpressApplication } from '@nestjs/platform-express';
import request from 'supertest';
import { createTestApp, healthyProbes, SlowController } from './helpers';

describe('health and request plumbing (e2e)', () => {
  let app: NestExpressApplication | undefined;

  afterEach(async () => {
    await app?.close();
    app = undefined;
  });

  it('health_live_ok', async () => {
    app = await createTestApp();
    await app.init();
    const res = await request(app.getHttpServer()).get('/api/v1/health/live').expect(200);
    expect(res.body).toEqual({ status: 'ok' });
    await request(app.getHttpServer()).get('/health').expect(200);
  });

  it('health_ready_reports_all_three_checks', async () => {
    app = await createTestApp();
    await app.init();
    const res = await request(app.getHttpServer()).get('/api/v1/health/ready').expect(200);
    expect(res.body).toEqual({
      status: 'ok',
      checks: { pg: 'up', redis: 'up', engine: 'up' },
    });
  });

  it('health_ready_503_when_pg_down', async () => {
    app = await createTestApp({ ...healthyProbes, pg: () => Promise.reject(new Error('down')) });
    await app.init();
    const res = await request(app.getHttpServer()).get('/api/v1/health/ready').expect(503);
    expect(res.body.checks).toEqual({ pg: 'down', redis: 'up', engine: 'up' });
    await request(app.getHttpServer()).get('/api/v1/health/live').expect(200);
  });

  it('health_ready_is_cached_for_two_seconds', async () => {
    const pg = jest.fn(() => Promise.resolve());
    app = await createTestApp({ ...healthyProbes, pg });
    await app.init();
    await request(app.getHttpServer()).get('/api/v1/health/ready').expect(200);
    await request(app.getHttpServer()).get('/api/v1/health/ready').expect(200);
    expect(pg).toHaveBeenCalledTimes(1);
  });

  it('request_id_propagated', async () => {
    app = await createTestApp();
    await app.init();
    const echoed = await request(app.getHttpServer())
      .get('/api/v1/health/live')
      .set('x-request-id', 'req-abc-123')
      .expect(200);
    expect(echoed.headers['x-request-id']).toBe('req-abc-123');

    const generated = await request(app.getHttpServer()).get('/api/v1/health/live');
    expect(generated.headers['x-request-id']).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
    );

    const missing = await request(app.getHttpServer())
      .get('/api/v1/nope')
      .set('x-request-id', 'req-404')
      .expect(404);
    expect(missing.headers['content-type']).toContain('application/problem+json');
    expect(missing.body.request_id).toBe('req-404');
  });

  it('sets_security_headers_and_cors', async () => {
    app = await createTestApp();
    await app.init();
    const res = await request(app.getHttpServer())
      .get('/api/v1/health/live')
      .set('origin', 'http://localhost:3000');
    expect(res.headers['x-content-type-options']).toBe('nosniff');
    expect(res.headers['access-control-allow-origin']).toBe('http://localhost:3000');
    expect(res.headers['access-control-allow-credentials']).toBe('true');
  });

  it('sigterm_drains_inflight_request', async () => {
    app = await createTestApp(healthyProbes, [SlowController]);
    await app.listen(0);
    const url = await app.getUrl();

    const inflight = fetch(`${url}/api/v1/slow`).then(async (r) => ({
      status: r.status,
      body: await r.json(),
    }));
    await new Promise((resolve) => setTimeout(resolve, 100));
    // Same code path the SIGTERM shutdown hook runs: close() drains before resolving.
    const closing = app.close();
    expect(await inflight).toEqual({ status: 200, body: { done: true } });
    await closing;
    app = undefined;
    await expect(fetch(`${url}/api/v1/slow`)).rejects.toThrow();
  });
});
