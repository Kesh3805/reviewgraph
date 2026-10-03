import { Public } from '../src/auth/public.decorator';
import { Controller, Get, type INestApplication, type Type } from '@nestjs/common';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { Test, type TestingModuleBuilder } from '@nestjs/testing';
import RedisMock from 'ioredis-mock';
import { AppModule } from '../src/app.module';
import { configureApp } from '../src/app.setup';
import { REDIS } from '../src/common/redis.module';
import { APP_CONFIG } from '../src/config/config.module';
import { parseEnv, type Env } from '../src/config/env.schema';
import { DependencyProbes, type HealthProbes } from '../src/health/health.probes';

export const VALID_ENV: NodeJS.ProcessEnv = {
  NODE_ENV: 'test',
  PORT: '8080',
  DATABASE_URL: 'postgres://u:p@127.0.0.1:1/db',
  REDIS_URL: 'redis://127.0.0.1:1',
  ENGINE_INTERNAL_URL: 'http://127.0.0.1:1',
  SERVICE_JWT_SECRET: 's'.repeat(32),
  SESSION_JWT_SECRET: 't'.repeat(32),
  TOKEN_CACHE_KEY: Buffer.alloc(32, 1).toString('base64'),
  WEB_ORIGIN: 'http://localhost:3000',
  GITHUB_ENABLED: 'false',
};

export function testConfig(overrides: NodeJS.ProcessEnv = {}): Env {
  return parseEnv({ ...VALID_ENV, ...overrides });
}

export const healthyProbes: HealthProbes = {
  pg: () => Promise.resolve(),
  redis: () => Promise.resolve(),
  engine: () => Promise.resolve(),
};

export async function createTestApp(
  probes: HealthProbes = healthyProbes,
  extraControllers: Type<unknown>[] = [],
  options: {
    redis?: unknown;
    env?: NodeJS.ProcessEnv;
    webhookBodyLimit?: string;
    configure?: (builder: TestingModuleBuilder) => TestingModuleBuilder;
  } = {},
): Promise<NestExpressApplication> {
  let builder = Test.createTestingModule({
    imports: [AppModule],
    controllers: extraControllers,
  })
    .overrideProvider(APP_CONFIG)
    .useValue(testConfig(options.env))
    .overrideProvider(REDIS)
    .useValue(options.redis ?? new RedisMock())
    .overrideProvider(DependencyProbes)
    .useValue(probes);
  if (options.configure) builder = options.configure(builder);
  const moduleRef = await builder.compile();
  const app = moduleRef.createNestApplication<NestExpressApplication>({ bodyParser: false });
  configureApp(app as INestApplication, 'http://localhost:3000', {
    webhookBodyLimit: options.webhookBodyLimit,
  });
  return app;
}

@Public()
@Controller('slow')
export class SlowController {
  @Get()
  async slow(): Promise<{ done: true }> {
    await new Promise((resolve) => setTimeout(resolve, 400));
    return { done: true };
  }
}
