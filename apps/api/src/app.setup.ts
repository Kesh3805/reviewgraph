import { RequestMethod, type INestApplication } from '@nestjs/common';
import type { NestExpressApplication } from '@nestjs/platform-express';
import helmet from 'helmet';
import { ProblemFilter } from './common/problem.filter';

export const JSON_BODY_LIMIT = '1mb';
/** Raw-body limit for the webhook route (registered by the webhooks module, GH tasks). */
export const WEBHOOK_BODY_LIMIT = '25mb';

/** HTTP-level setup shared by `main.ts` and the e2e tests. */
export function configureApp(app: INestApplication, webOrigin: string): void {
  const express = app as NestExpressApplication;
  // The Nest default parser is disabled in main/test bootstrap so the limit is explicit.
  express.useBodyParser('json', { limit: JSON_BODY_LIMIT });
  express.useBodyParser('urlencoded', { limit: JSON_BODY_LIMIT, extended: true });
  app.use(helmet());
  app.enableCors({ origin: webOrigin, credentials: true });
  // Internal service-to-service routes live under /internal/** (API-005), outside the public prefix.
  app.setGlobalPrefix('api/v1', {
    exclude: ['health', { path: 'internal/{*path}', method: RequestMethod.ALL }],
  });
  app.useGlobalFilters(new ProblemFilter());
  app.enableShutdownHooks();
}
