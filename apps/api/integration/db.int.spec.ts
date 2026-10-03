import { sql } from 'kysely';
import { createKysely, createPool } from '../src/db/kysely.provider';
import { runInTx } from '../src/db/tx';

describe('withTx (integration)', () => {
  const pool = createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max: 2 });
  const db = createKysely(pool);

  afterAll(async () => {
    await db.destroy();
  });

  const read = async (trx: typeof db): Promise<string | null> => {
    const { rows } = await sql<{
      v: string | null;
    }>`select current_setting('app.organization_id', true) as v`.execute(trx);
    return rows[0]?.v ?? null;
  };

  it('with_tx_sets_org_setting', async () => {
    const org = '0190f3a2-0000-7000-8000-000000000001';
    const seen = await runInTx(db, org, (trx) => read(trx));
    expect(seen).toBe(org);
  });

  it('does not leak the setting across pooled connections', async () => {
    await runInTx(db, '0190f3a2-0000-7000-8000-000000000002', (trx) => read(trx));
    // pool max is 2, so this reuses a connection that previously carried the setting.
    const after = await db.transaction().execute((trx) => read(trx));
    expect(after === null || after === '').toBe(true);
  });

  it('sets an empty setting when there is no tenant', async () => {
    expect(await runInTx(db, undefined, (trx) => read(trx))).toBe('');
  });

  it('applies the configured pool settings', async () => {
    const { rows } = await sql<{
      app: string;
      timeout: string;
    }>`select current_setting('application_name') as app, current_setting('statement_timeout') as timeout`.execute(
      db,
    );
    expect(rows[0]).toEqual({ app: 'rg-api', timeout: '15s' });
  });
});
