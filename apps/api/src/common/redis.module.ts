import { Global, Inject, Module, OnApplicationShutdown } from '@nestjs/common';
import { Redis } from 'ioredis';
import { APP_CONFIG, type AppConfig } from '../config/config.module';

export const REDIS = Symbol('REDIS');

/** Shared Redis client for caches, locks and idempotency keys (no source, graphs or ASTs). */
@Global()
@Module({
  providers: [
    {
      provide: REDIS,
      inject: [APP_CONFIG],
      useFactory: (config: AppConfig): Redis => {
        const client = new Redis(config.REDIS_URL, {
          lazyConnect: true,
          maxRetriesPerRequest: 1,
          connectTimeout: 1500,
          retryStrategy: (times) => Math.min(times * 200, 2000),
        });
        // Connection errors surface per command; never crash the process on a socket error.
        client.on('error', () => undefined);
        return client;
      },
    },
  ],
  exports: [REDIS],
})
export class RedisModule implements OnApplicationShutdown {
  constructor(@Inject(REDIS) private readonly redis: Redis) {}

  async onApplicationShutdown(): Promise<void> {
    if (this.redis.status === 'wait' || this.redis.status === 'end') return;
    await Promise.allSettled([this.redis.quit()]);
    this.redis.disconnect();
  }
}
