import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { buildOpenApiDocument } from '../src/openapi';
import { createTestApp } from './helpers';

const SNAPSHOT = resolve(__dirname, '../../../packages/contracts/openapi/api.json');

const renderDocument = (app: NestExpressApplication): string =>
  `${JSON.stringify(buildOpenApiDocument(app), null, 2)}\n`;

const readSnapshot = (rendered: string): string => {
  if (!existsSync(SNAPSHOT) || process.env.RG_UPDATE_SNAPSHOTS === '1') {
    mkdirSync(dirname(SNAPSHOT), { recursive: true });
    writeFileSync(SNAPSHOT, rendered);
  }
  return readFileSync(SNAPSHOT, 'utf8');
};

describe('OpenAPI document', () => {
  let app: NestExpressApplication;

  beforeAll(async () => {
    app = await createTestApp();
    await app.init();
  });
  afterAll(() => app.close());

  it('openapi_snapshot', () => {
    const rendered = renderDocument(app);
    expect(readSnapshot(rendered)).toBe(rendered);
  });

  it('lists every repositories route (PRD section 106)', () => {
    const paths = buildOpenApiDocument(app).paths;
    const has = (path: string, method: string): boolean =>
      Boolean((paths[path] as Record<string, unknown> | undefined)?.[method]);
    expect(has('/api/v1/repositories', 'post')).toBe(true);
    expect(has('/api/v1/repositories', 'get')).toBe(true);
    expect(has('/api/v1/repositories/{repoId}', 'get')).toBe(true);
    expect(has('/api/v1/repositories/{repoId}/initialize', 'post')).toBe(true);
    expect(has('/api/v1/repositories/{repoId}/status', 'get')).toBe(true);
    expect(has('/api/v1/repositories/{repoId}/profile', 'get')).toBe(true);
    expect(has('/api/v1/repositories/{repoId}/graph/rebuild', 'post')).toBe(true);
    expect(has('/api/v1/repositories/{repoId}/settings', 'patch')).toBe(true);
    expect(has('/api/v1/auth/me', 'get')).toBe(true);
  });

  it('lists every reviews route (API-009)', () => {
    const paths = buildOpenApiDocument(app).paths;
    const has = (path: string, method: string): boolean =>
      Boolean((paths[path] as Record<string, unknown> | undefined)?.[method]);
    expect(has('/api/v1/repositories/{repoId}/pull-requests', 'get')).toBe(true);
    expect(has('/api/v1/pull-requests/{pullRequestId}', 'get')).toBe(true);
    expect(has('/api/v1/pull-requests/{pullRequestId}/review', 'post')).toBe(true);
    expect(has('/api/v1/pull-requests/{pullRequestId}/reviews', 'get')).toBe(true);
    expect(has('/api/v1/pull-requests/{pullRequestId}/reviews/{reviewId}', 'get')).toBe(true);
    expect(has('/api/v1/reviews/{reviewId}/cancel', 'post')).toBe(true);
  });

  it('lists every findings route (API-010)', () => {
    const paths = buildOpenApiDocument(app).paths;
    const has = (path: string, method: string): boolean =>
      Boolean((paths[path] as Record<string, unknown> | undefined)?.[method]);
    expect(has('/api/v1/reviews/{reviewId}/findings', 'get')).toBe(true);
    expect(has('/api/v1/findings/{findingId}', 'get')).toBe(true);
    expect(has('/api/v1/findings/{findingId}/trace', 'get')).toBe(true);
  });
});
