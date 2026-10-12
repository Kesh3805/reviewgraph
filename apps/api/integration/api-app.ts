import type { NestExpressApplication } from '@nestjs/platform-express';
import type { TestingModuleBuilder } from '@nestjs/testing';
import request from 'supertest';
import { SessionService } from '../src/auth/session.service';
import { createTestApp } from '../test/helpers';
import { asUser, CSRF_HEADERS, testSessions } from '../test/helpers/tenancy-probe';

/**
 * The full AppModule against the integration database, with the RLS-enforced role and cookie
 * sessions stubbed (`rg_session=test:<userId>`); every other guard is real.
 */
export async function createApiApp(
  configure?: (builder: TestingModuleBuilder) => TestingModuleBuilder,
  env: NodeJS.ProcessEnv = {},
): Promise<NestExpressApplication> {
  const app = await createTestApp(undefined, [], {
    env: { DATABASE_URL: process.env.RG_TEST_DATABASE_URL!, DB_APP_ROLE: 'rg_api', ...env },
    configure: (builder) => {
      const b = builder.overrideProvider(SessionService).useValue(testSessions);
      return configure ? configure(b) : b;
    },
  });
  await app.init();
  return app;
}

/** Authenticated request helpers for one user. */
export function as(app: NestExpressApplication, userId: string) {
  const agent = () => request(app.getHttpServer());
  return {
    get: (path: string) => agent().get(`/api/v1${path}`).set('cookie', asUser(userId)),
    post: (path: string, body?: object) =>
      agent()
        .post(`/api/v1${path}`)
        .set('cookie', asUser(userId))
        .set(CSRF_HEADERS)
        .send(body ?? {}),
    patch: (path: string, body?: object) =>
      agent()
        .patch(`/api/v1${path}`)
        .set('cookie', asUser(userId))
        .set(CSRF_HEADERS)
        .send(body ?? {}),
  };
}
