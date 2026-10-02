import 'reflect-metadata';
import { NestFactory } from '@nestjs/core';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { AppModule } from './app.module';
import { configureApp } from './app.setup';
import { loadDotenv } from './config/config.module';
import { JsonLoggerService } from './telemetry/json-logger';
import { ConfigError, EX_CONFIG, parseEnv, type Env } from './config/env.schema';

/** Fails fast on invalid configuration: exit 78 with value-free issues. */
function loadConfig(): Env {
  loadDotenv();
  try {
    return parseEnv(process.env);
  } catch (err) {
    if (err instanceof ConfigError) {
      console.error(err.message);
      process.exit(EX_CONFIG);
    }
    throw err;
  }
}

async function bootstrap(): Promise<void> {
  const config = loadConfig();
  const logger = new JsonLoggerService();
  const app = await NestFactory.create<NestExpressApplication>(AppModule, {
    bodyParser: false,
    logger,
  });
  configureApp(app, config.WEB_ORIGIN);
  await app.listen(config.PORT);
  logger.log(`api listening on :${config.PORT}`, 'Bootstrap');
}

void bootstrap();
