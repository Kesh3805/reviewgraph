import { sql, type Kysely, type Transaction } from 'kysely';
import type { DB } from './generated';

export type Tx = Transaction<DB>;

export interface TxOptions {
  /**
   * Role assumed for the transaction (`SET LOCAL ROLE`). The application login normally IS the
   * RLS-enforced role; in development the login is a superuser, so `DB_APP_ROLE=rg_api` makes the
   * same policies apply there.
   */
  role?: string;
}

/**
 * Runs `fn` in a transaction that always sets `app.organization_id` (transaction-local, so a
 * pooled connection never carries a tenant across requests). With no organization the setting is
 * the empty string and RLS returns no rows (fail closed).
 */
export async function runInTx<T>(
  db: Kysely<DB>,
  orgId: string | null | undefined,
  fn: (trx: Tx) => Promise<T>,
  options: TxOptions = {},
): Promise<T> {
  return db.transaction().execute(async (trx) => {
    if (options.role) await sql`select set_config('role', ${options.role}, true)`.execute(trx);
    await sql`select set_config('app.organization_id', ${orgId ?? ''}, true)`.execute(trx);
    return fn(trx);
  });
}
