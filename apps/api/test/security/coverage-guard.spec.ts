import type { NestExpressApplication } from '@nestjs/platform-express';
import { buildOpenApiDocument } from '../../src/openapi';
import { createTestApp } from '../helpers';
import { ISOLATION_MATRIX, TENANT_ID_PARAMS } from './isolation-matrix';

describe('tenant isolation coverage guard (SEC-001)', () => {
  let app: NestExpressApplication;

  beforeAll(async () => {
    app = await createTestApp();
    await app.init();
  });
  afterAll(() => app.close());

  const routes = (): string[] =>
    Object.entries(buildOpenApiDocument(app).paths).flatMap(([path, ops]) =>
      Object.keys(ops as object)
        .filter((m) => ['get', 'post', 'put', 'patch', 'delete'].includes(m))
        .map((m) => `${m.toUpperCase()} ${path}`),
    );

  it('every_route_has_isolation_matrix_entry', () => {
    const missing = routes().filter((r) => !(r in ISOLATION_MATRIX));
    // Add each missing route to test/security/isolation-matrix.ts with its isolation behavior.
    expect(missing).toEqual([]);
    const stale = Object.keys(ISOLATION_MATRIX).filter((r) => !routes().includes(r));
    expect(stale).toEqual([]);
  });

  it('every route with a tenant id param is a foreign_id_404 route', () => {
    for (const route of routes()) {
      const hasTenantParam = TENANT_ID_PARAMS.some((p) => route.includes(`{${p}}`));
      if (hasTenantParam)
        expect([route, ISOLATION_MATRIX[route]]).toEqual([route, 'foreign_id_404']);
    }
  });

  it('internal routes are service-auth only', () => {
    for (const route of routes().filter((r) => r.includes(' /internal/'))) {
      expect([route, ISOLATION_MATRIX[route]]).toEqual([route, 'service_auth_only']);
    }
  });
});
