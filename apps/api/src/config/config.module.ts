import { Global, Module } from '@nestjs/common';
import { parseEnv, type Env } from './env.schema';

export const APP_CONFIG = Symbol('APP_CONFIG');
export type AppConfig = Env;

/** Loads `.env` (if present) without overriding variables already set in the process. */
export function loadDotenv(path = '.env'): void {
  try {
    process.loadEnvFile(path);
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err;
  }
}

@Global()
@Module({
  providers: [{ provide: APP_CONFIG, useFactory: (): AppConfig => parseEnv(process.env) }],
  exports: [APP_CONFIG],
})
export class ConfigModule {}
