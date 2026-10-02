import { RequestMethod, type INestApplication } from '@nestjs/common';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { raw } from 'express';
import helmet from 'helmet';
import { ProblemFilter } from './common/problem.filter';

export const JSON_BODY_LIMIT = '1mb';
/** Raw-body limit for the webhook route (GH-002). */
export const WEBHOOK_BODY_LIMIT = '25mb';

export const GITHUB_WEBHOOK_PATH = '/api/v1/webhooks/github';

export interface AppSetupOptions {
  /** Raw-body limit for the webhook route; defaults to 25 MiB. */
  webhookBodyLimit?: string;
}

/** HTTP-level setup shared by `main.ts` and the e2e tests. */
export function configureApp(
  app: INestApplication,
  webOrigin: string,
  options: AppSetupOptions = {},
): void {
  const express = app as NestExpressApplication;
  // The Nest default parser is disabled in main/test bootstrap so the limit is explicit.
  // The webhook HMAC is computed over the exact bytes received, so this route gets a raw
  // parser (registered first; the JSON parsers skip a request whose body is already read).
  app.use(
    GITHUB_WEBHOOK_PATH,
    raw({ type: () => true, limit: options.webhookBodyLimit ?? WEBHOOK_BODY_LIMIT }),
  );
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
