import { Inject, Injectable, OnApplicationShutdown } from '@nestjs/common';
import { Redis } from 'ioredis';
import { Pool } from 'pg';
import { APP_CONFIG, type AppConfig } from '../config/config.module';

export const HEALTH_PROBES = Symbol('HEALTH_PROBES');

/** Each probe resolves when the dependency is healthy and rejects otherwise. */
export interface HealthProbes {
  pg(): Promise<void>;
  redis(): Promise<void>;
  engine(): Promise<void>;
}

const PROBE_TIMEOUT_MS = 1500;

/**
 * Lightweight dependency probes. These hold their own tiny connections so readiness does not
 * compete with the business pools (the typed DbModule arrives with API-002).
 */
@Injectable()
export class DependencyProbes implements HealthProbes, OnApplicationShutdown {
  private readonly pool: Pool;
  private readonly redisClient: Redis;
  private readonly engineUrl: string;

  constructor(@Inject(APP_CONFIG) config: AppConfig) {
    this.pool = new Pool({
      connectionString: config.DATABASE_URL,
      max: 1,
      connectionTimeoutMillis: PROBE_TIMEOUT_MS,
      idleTimeoutMillis: 10_000,
    });
    this.pool.on('error', () => undefined);
    this.redisClient = new Redis(config.REDIS_URL, {
      lazyConnect: true,
      maxRetriesPerRequest: 0,
      enableOfflineQueue: false,
      connectTimeout: PROBE_TIMEOUT_MS,
      retryStrategy: (times) => Math.min(times * 200, 2000),
    });
    this.redisClient.on('error', () => undefined);
    this.engineUrl = new URL('/internal/v1/health', config.ENGINE_INTERNAL_URL).toString();
  }

  async pg(): Promise<void> {
    await withTimeout(this.pool.query('SELECT 1'));
  }

  async redis(): Promise<void> {
    if (this.redisClient.status === 'wait' || this.redisClient.status === 'end') {
      await withTimeout(this.redisClient.connect());
    }
    const reply = await withTimeout(this.redisClient.ping());
    if (reply !== 'PONG') throw new Error('unexpected redis reply');
  }

  async engine(): Promise<void> {
    const res = await fetch(this.engineUrl, { signal: AbortSignal.timeout(PROBE_TIMEOUT_MS) });
    if (!res.ok) throw new Error(`engine health returned ${res.status}`);
  }

  /** Closes the pools; runs after the HTTP server has drained. */
  async onApplicationShutdown(): Promise<void> {
    await Promise.allSettled([this.pool.end(), this.redisClient.quit()]);
    this.redisClient.disconnect();
  }
}

function withTimeout<T>(promise: Promise<T>, ms = PROBE_TIMEOUT_MS): Promise<T> {
  let timer: NodeJS.Timeout;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error('probe timeout')), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}
