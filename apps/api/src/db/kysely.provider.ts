import { Kysely, PostgresDialect } from 'kysely';
import { Pool } from 'pg';
import { setGauge } from '../common/metrics';
import type { DB } from './generated';

export const POOL_DEFAULTS = {
  max: 20,
  idleTimeoutMillis: 30_000,
  /** Connection acquire timeout: an exhausted pool fails the request with 503 after this. */
  connectionTimeoutMillis: 5_000,
  statementTimeoutMs: 15_000,
} as const;

export interface PoolOptions {
  connectionString: string;
  max?: number;
  connectionTimeoutMillis?: number;
  statementTimeoutMs?: number;
}

/** `pg.Pool` with the API-002 settings; the gauges feed `db_pool_in_use` / `db_pool_waiting`. */
export function createPool(opts: PoolOptions): Pool {
  const pool = new Pool({
    connectionString: opts.connectionString,
    max: opts.max ?? POOL_DEFAULTS.max,
    idleTimeoutMillis: POOL_DEFAULTS.idleTimeoutMillis,
    connectionTimeoutMillis: opts.connectionTimeoutMillis ?? POOL_DEFAULTS.connectionTimeoutMillis,
    // Set per connection by the driver (startup parameters), so it holds for every statement.
    statement_timeout: opts.statementTimeoutMs ?? POOL_DEFAULTS.statementTimeoutMs,
    application_name: 'rg-api',
  });
  // An idle client erroring (server restart) must not crash the process.
  pool.on('error', () => undefined);
  const publish = (): void => {
    setGauge('db_pool_in_use', pool.totalCount - pool.idleCount);
    setGauge('db_pool_waiting', pool.waitingCount);
  };
  for (const event of ['connect', 'acquire', 'release', 'remove'] as const) {
    pool.on(event, publish);
  }
  return pool;
}

/** Column names stay snake_case (no CamelCasePlugin) to match the Rust side and the contracts. */
export function createKysely(pool: Pool): Kysely<DB> {
  return new Kysely<DB>({ dialect: new PostgresDialect({ pool }) });
}
