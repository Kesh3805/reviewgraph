import { Global, Inject, Injectable, Module } from '@nestjs/common';
import { APP_GUARD } from '@nestjs/core';
import { hostname } from 'node:os';
import type { Redis } from 'ioredis';
import { REDIS } from '../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { CloneCredentialsController } from './clone-credentials.controller';
import { SERVICE_JTI_STORE, SERVICE_KEY_RING, ServiceAuthGuard } from './service-auth.guard';
import {
  RedisJtiStore,
  ServiceKeyRing,
  signServiceToken,
  type ServiceAudience,
  type ServiceScope,
} from './service-token';

/** Builds the key ring from `SERVICE_JWT_KEYS`, falling back to `SERVICE_JWT_SECRET`. */
export function serviceKeyRingFromConfig(config: AppConfig): ServiceKeyRing {
  if (config.SERVICE_JWT_KEYS) return ServiceKeyRing.parse(config.SERVICE_JWT_KEYS);
  return ServiceKeyRing.from([
    { kid: 'default', secret: Buffer.from(config.SERVICE_JWT_SECRET, 'utf8') },
  ]);
}

/** Mints the short-lived bearer tokens the API presents to other services (e.g. the engine). */
@Injectable()
export class ServiceTokenService {
  private readonly sub = `rg-api:${hostname()}:${process.pid}`;

  constructor(@Inject(SERVICE_KEY_RING) private readonly keys: ServiceKeyRing) {}

  issue(opts: {
    aud: ServiceAudience;
    scope: ServiceScope[];
    org?: string;
    repo?: string;
  }): Promise<string> {
    return signServiceToken(this.keys, { iss: 'rg-api', sub: this.sub, ...opts });
  }
}

/**
 * Service-to-service auth (API-005). The guard is global but only acts on `/internal/**`.
 */
@Global()
@Module({
  controllers: [CloneCredentialsController],
  providers: [
    {
      provide: SERVICE_KEY_RING,
      inject: [APP_CONFIG],
      useFactory: serviceKeyRingFromConfig,
    },
    {
      provide: SERVICE_JTI_STORE,
      inject: [REDIS],
      useFactory: (redis: Redis) => new RedisJtiStore(redis),
    },
    { provide: APP_GUARD, useClass: ServiceAuthGuard },
    ServiceTokenService,
  ],
  exports: [SERVICE_KEY_RING, SERVICE_JTI_STORE, ServiceTokenService],
})
export class InternalModule {}
