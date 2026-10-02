import { Module } from '@nestjs/common';
import type { Redis } from 'ioredis';
import { REDIS } from '../../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import { GithubAppAuth, loadPrivateKey } from './app-auth.service';
import { InstallationTokenCache } from './token-cache';

export const GITHUB_APP_AUTH = Symbol('GITHUB_APP_AUTH');

/** Null when `GITHUB_ENABLED=false` (CLI-only local development). */
export function createGithubAppAuth(config: AppConfig, redis: Redis): GithubAppAuth | null {
  if (!config.GITHUB_ENABLED) return null;
  if (!config.GITHUB_APP_ID) throw new Error('GITHUB_APP_ID is required when GitHub is enabled');
  return new GithubAppAuth({
    appId: config.GITHUB_APP_ID,
    // A key that does not parse throws here, which fails the boot.
    privateKey: loadPrivateKey(config),
    apiUrl: config.GITHUB_API_URL,
    cache: new InstallationTokenCache(redis, Buffer.from(config.TOKEN_CACHE_KEY, 'base64')),
  });
}

@Module({
  providers: [
    {
      provide: GITHUB_APP_AUTH,
      inject: [APP_CONFIG, REDIS],
      useFactory: createGithubAppAuth,
    },
  ],
  exports: [GITHUB_APP_AUTH],
})
export class GithubModule {}
