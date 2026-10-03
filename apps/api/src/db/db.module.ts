import { Global, Inject, Injectable, Module, OnApplicationShutdown } from '@nestjs/common';
import type { Kysely } from 'kysely';
import type { Pool } from 'pg';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import type { DB } from './generated';
import { createKysely, createPool } from './kysely.provider';
import { runInTx, type Tx } from './tx';

export const PG_POOL = Symbol('PG_POOL');
/** `Kysely<DB>` on the shared pool. */
export const DB_TOKEN = Symbol('DB');

/** Typed query access plus the tenant-scoped transaction helper. */
@Injectable()
export class DbService implements OnApplicationShutdown {
  constructor(
    @Inject(DB_TOKEN) readonly db: Kysely<DB>,
    @Inject(APP_CONFIG) private readonly config: AppConfig,
  ) {}

  /**
   * `withTx(orgId, fn)` or `withTx(fn)` (no tenant: RLS then sees no tenant rows). The setting
   * `app.organization_id` is always written, transaction-locally.
   */
  withTx<T>(fn: (trx: Tx) => Promise<T>): Promise<T>;
  withTx<T>(orgId: string | null | undefined, fn: (trx: Tx) => Promise<T>): Promise<T>;
  withTx<T>(
    first: string | null | undefined | ((trx: Tx) => Promise<T>),
    second?: (trx: Tx) => Promise<T>,
  ): Promise<T> {
    const [orgId, fn] = typeof first === 'function' ? [undefined, first] : [first, second!];
    return runInTx(this.db, orgId, fn, { role: this.config.DB_APP_ROLE });
  }

  async onApplicationShutdown(): Promise<void> {
    await this.db.destroy();
  }
}

@Global()
@Module({
  providers: [
    {
      provide: PG_POOL,
      inject: [APP_CONFIG],
      useFactory: (config: AppConfig): Pool =>
        createPool({ connectionString: config.DATABASE_URL, max: config.DB_POOL_MAX }),
    },
    {
      provide: DB_TOKEN,
      inject: [PG_POOL],
      useFactory: (pool: Pool): Kysely<DB> => createKysely(pool),
    },
    DbService,
  ],
  exports: [PG_POOL, DB_TOKEN, DbService],
})
export class DbModule {}
