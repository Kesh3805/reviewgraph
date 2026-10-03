import { Module } from '@nestjs/common';
import type { Redis } from 'ioredis';
import { REDIS } from '../../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import { RepositoriesModule } from '../../repositories/repository-settings.port';
import { GithubAppAuth, loadPrivateKey } from './app-auth.service';
import { GithubCommandAcknowledger } from './command-reaction';
import { GithubEventNormalizer } from './event-normalizer.service';
import { GITHUB_APP_AUTH } from './github.tokens';
import { GITHUB_PERMISSIONS_STATUS, GithubPermissionsMonitor } from './permissions-monitor';
import { ACTOR_PERMISSION_LOOKUP, GithubActorPermissions } from './permissions';
import { InstallationTokenCache } from './token-cache';

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
  imports: [RepositoriesModule],
  providers: [
    {
      provide: GITHUB_APP_AUTH,
      inject: [APP_CONFIG, REDIS],
      useFactory: createGithubAppAuth,
    },
    GithubActorPermissions,
    { provide: ACTOR_PERMISSION_LOOKUP, useExisting: GithubActorPermissions },
    GithubEventNormalizer,
    GithubCommandAcknowledger,
    GithubPermissionsMonitor,
    { provide: GITHUB_PERMISSIONS_STATUS, useExisting: GithubPermissionsMonitor },
  ],
  exports: [
    GITHUB_APP_AUTH,
    GithubEventNormalizer,
    GithubCommandAcknowledger,
    GithubPermissionsMonitor,
    GITHUB_PERMISSIONS_STATUS,
  ],
})
export class GithubModule {}
