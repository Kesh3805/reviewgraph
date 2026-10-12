import type { KeyObject } from 'node:crypto';
import { Module } from '@nestjs/common';
import type { Redis } from 'ioredis';
import { REDIS } from '../../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import { RepositoriesModule } from '../../repositories/repositories.module';
import { GithubAppAuth, loadPrivateKey } from './app-auth.service';
import { GithubCommandAcknowledger } from './command-reaction';
import { GithubEventNormalizer } from './event-normalizer.service';
import { GITHUB_APP_AUTH } from './github.tokens';
import { JOB_CANCELLER, NoopJobCanceller } from '../../common/job-canceller';
import { INSTALLATION_GATE, InstallationService } from './installation.service';
import { GITHUB_PERMISSIONS_STATUS, GithubPermissionsMonitor } from './permissions-monitor';
import { ACTOR_PERMISSION_LOOKUP, GithubActorPermissions } from './permissions';
import { GithubProviderRegistration } from './provider-registration';
import { GithubRepositoryProvider } from './repository-provider';
import { GithubReviewPublisher } from './review-publisher';
import { InstallationTokenCache } from './token-cache';
import { scrubPrivateKeyFromEnv } from '../../security/secret-source';
import { TokenCipher } from '../../security/token-cipher';

/** Null when `GITHUB_ENABLED=false` (CLI-only local development). */
export function createGithubAppAuth(config: AppConfig, redis: Redis): GithubAppAuth | null {
  if (!config.GITHUB_ENABLED) return null;
  if (!config.GITHUB_APP_ID) throw new Error('GITHUB_APP_ID is required when GitHub is enabled');
  return new GithubAppAuth({
    appId: config.GITHUB_APP_ID,
    // A key that does not parse throws here, which fails the boot.
    privateKey: loadAppKeyOnce(config),
    apiUrl: config.GITHUB_API_URL,
    cache: new InstallationTokenCache(
      redis,
      TokenCipher.fromConfig(config.TOKEN_CACHE_KEY, config.TOKEN_CACHE_KEY_PREVIOUS),
    ),
  });
}

/** Parses the key and removes the inline PEM from `process.env` (SEC-005). */
function loadAppKeyOnce(config: AppConfig): KeyObject {
  const key = loadPrivateKey(config);
  if (process.env.GITHUB_APP_PRIVATE_KEY === config.GITHUB_APP_PRIVATE_KEY) {
    scrubPrivateKeyFromEnv();
  }
  return key;
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
    // API-007 replaces the no-op with the queue adapter.
    { provide: JOB_CANCELLER, useClass: NoopJobCanceller },
    InstallationService,
    { provide: INSTALLATION_GATE, useExisting: InstallationService },
    { provide: GITHUB_PERMISSIONS_STATUS, useExisting: GithubPermissionsMonitor },
    GithubRepositoryProvider,
    GithubReviewPublisher,
    GithubProviderRegistration,
  ],
  exports: [
    GITHUB_APP_AUTH,
    GithubEventNormalizer,
    GithubCommandAcknowledger,
    GithubPermissionsMonitor,
    InstallationService,
    INSTALLATION_GATE,
    GITHUB_PERMISSIONS_STATUS,
    GithubRepositoryProvider,
    GithubReviewPublisher,
  ],
})
export class GithubModule {}
