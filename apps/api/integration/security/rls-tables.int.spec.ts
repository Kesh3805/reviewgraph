import { sql, type Kysely } from 'kysely';
import type { DB } from '../../src/db/generated';
import { createKysely, createPool } from '../../src/db/kysely.provider';
import { runInTx } from '../../src/db/tx';
import { adminDb, cleanup } from '../seed';
import { buildTenant, type Tenant } from './tenancy.fixture';

/**
 * Database-level isolation (SEC-001): every table with an `organization_id` column (plus
 * `organizations` itself) is enumerated from the catalog and checked as the RLS-enforced roles.
 * A new tenant table without forced RLS fails here by name.
 */
describe('RLS on every tenant table (SEC-001)', () => {
  const admin = adminDb();
  // One connection: every transaction below reuses it, which also proves no carry-over.
  const appDb = createKysely(
    createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max: 1 }),
  );
  let a: Tenant;
  let b: Tenant;
  let tables: string[];

  beforeAll(async () => {
    [a, b] = await Promise.all([buildTenant(admin), buildTenant(admin)]);
    const { rows } = await sql<{ table_name: string }>`
      select c.table_name::text as table_name from information_schema.columns c
      join information_schema.tables t
        on t.table_schema = c.table_schema and t.table_name = c.table_name
      where c.table_schema = 'public' and c.column_name = 'organization_id'
        and t.table_type = 'BASE TABLE'
      order by 1`.execute(admin);
    tables = [...rows.map((r) => r.table_name), 'organizations'];
  });

  afterAll(async () => {
    await cleanup(admin, [a.org.organizationId, b.org.organizationId], [a.owner, b.owner]);
    await admin.destroy();
    await appDb.destroy();
  });

  const orgColumn = (table: string) => (table === 'organizations' ? 'id' : 'organization_id');

  /** Runs `fn` as `role` with the tenant setting (or none). */
  const as = <T>(role: string, orgId: string | null, fn: (trx: Kysely<DB>) => Promise<T>) =>
    runInTx(appDb, orgId, (trx) => fn(trx as unknown as Kysely<DB>), { role });

  const countOf = (trx: Kysely<DB>, table: string, orgId: string) =>
    sql<{ n: string }>`select count(*)::text as n from ${sql.table(table)}
      where ${sql.ref(orgColumn(table))} = ${orgId}::uuid`
      .execute(trx)
      .then((r) => Number(r.rows[0]?.n ?? 0));

  it('runs as a role that is not a superuser', async () => {
    for (const role of ['rg_api', 'rg_engine']) {
      const { rows } = await as(role, null, (trx) =>
        sql<{ rolsuper: boolean; rolbypassrls: boolean }>`
          select rolsuper, rolbypassrls from pg_roles where rolname = current_user`.execute(trx),
      );
      expect(rows[0]).toEqual({ rolsuper: false, rolbypassrls: false });
    }
  });

  it('every_org_table_has_forced_rls', async () => {
    expect(tables.length).toBeGreaterThan(15);
    const { rows } = await sql<{ relname: string; rls: boolean; force: boolean; policies: string }>`
      select c.relname::text as relname, c.relrowsecurity as rls, c.relforcerowsecurity as force,
             (select count(*)::text from pg_policies p
               where p.schemaname = 'public' and p.tablename = c.relname) as policies
      from pg_class c
      where c.relnamespace = 'public'::regnamespace and c.relname = any(${tables})`.execute(admin);
    const byName = new Map(rows.map((r) => [r.relname, r]));
    for (const table of tables) {
      const row = byName.get(table);
      expect([table, row?.rls, row?.force, Number(row?.policies ?? 0) > 0]).toEqual([
        table,
        true,
        true,
        true,
      ]);
    }
  });

  it('rg_api_without_org_setting_sees_zero_rows', async () => {
    for (const role of ['rg_api', 'rg_engine']) {
      for (const table of tables) {
        const visible = await as(role, null, (trx) =>
          sql<{ n: string }>`select count(*)::text as n from ${sql.table(table)}`
            .execute(trx)
            .then((r) => Number(r.rows[0]?.n)),
        );
        expect([role, table, visible]).toEqual([role, table, 0]);
      }
    }
  });

  it('a tenant sees, updates and deletes only its own rows', async () => {
    let populated = 0;
    for (const table of tables) {
      // Org B's rows exist (as the superuser sees them) ...
      const total = await countOf(admin, table, b.org.organizationId);
      if (total > 0) populated++;
      // ... but org A sees none of them, and cannot touch them.
      expect([
        table,
        await as('rg_api', a.org.organizationId, (t) => countOf(t, table, b.org.organizationId)),
      ]).toEqual([table, 0]);
      const updated = await as('rg_api', a.org.organizationId, (trx) =>
        sql`update ${sql.table(table)} set ${sql.ref(orgColumn(table))} = ${sql.ref(orgColumn(table))}
            where ${sql.ref(orgColumn(table))} = ${b.org.organizationId}::uuid`.execute(trx),
      ).catch((err: Error) => err);
      // Either zero rows or a privilege error (append-only tables): never a change.
      if (!(updated instanceof Error))
        expect([table, Number(updated.numAffectedRows ?? 0n)]).toEqual([table, 0]);
      const deleted = await as('rg_api', a.org.organizationId, (trx) =>
        sql`delete from ${sql.table(table)} where ${sql.ref(orgColumn(table))} = ${b.org.organizationId}::uuid`.execute(
          trx,
        ),
      ).catch((err: Error) => err);
      if (!(deleted instanceof Error))
        expect([table, Number(deleted.numAffectedRows ?? 0n)]).toEqual([table, 0]);
      expect([table, await countOf(admin, table, b.org.organizationId)]).toEqual([table, total]);
    }
    // The fixture populates most tenant tables, so the checks above are not vacuous.
    expect(populated).toBeGreaterThanOrEqual(13);
  });

  it('rg_api_cross_org_insert_rejected_with_check', async () => {
    let checked = 0;
    for (const table of tables) {
      const has = await countOf(admin, table, b.org.organizationId);
      if (has === 0) continue;
      checked++;
      // Copy one of org B's rows and try to insert it while acting as org A.
      const { rows } = await sql<{ row: unknown }>`
        select to_jsonb(x) as row from ${sql.table(table)} x
        where ${sql.ref(`x.${orgColumn(table)}`)} = ${b.org.organizationId}::uuid limit 1`.execute(
        admin,
      );
      const result = await as('rg_api', a.org.organizationId, async (trx) => {
        await sql`insert into ${sql.table(table)}
                  select * from jsonb_populate_record(null::${sql.table(table)}, ${JSON.stringify(
                    rows[0]!.row,
                  )}::jsonb)`.execute(trx);
        return 'inserted';
      }).catch((err: Error) => err.message);
      expect([table, result]).toEqual([
        table,
        expect.stringMatching(/row-level security|permission denied/),
      ]);
    }
    expect(checked).toBeGreaterThanOrEqual(13);
  });

  it('jobs are visible across tenants only to worker transactions', async () => {
    const asWorker = await appDb.transaction().execute(async (trx) => {
      await sql`select set_config('role', 'rg_api', true)`.execute(trx);
      await sql`select set_config('app.organization_id', '', true)`.execute(trx);
      await sql`select set_config('app.job_worker', 'on', true)`.execute(trx);
      return countOf(trx as unknown as Kysely<DB>, 'jobs', b.org.organizationId);
    });
    expect(asWorker).toBeGreaterThan(0);
    // The opt-in is transaction-local: the next transaction on the same connection is a tenant.
    expect(
      await as('rg_api', a.org.organizationId, (t) => countOf(t, 'jobs', b.org.organizationId)),
    ).toBe(0);
  });

  it('pooled_connection_does_not_carry_org', async () => {
    expect(
      await as('rg_api', a.org.organizationId, (t) =>
        countOf(t, 'repositories', a.org.organizationId),
      ),
    ).toBe(1);
    const leaked = await appDb.transaction().execute(async (trx) => {
      await sql`select set_config('role', 'rg_api', true)`.execute(trx);
      const { rows } = await sql<{ org: string | null; worker: string | null }>`
        select current_setting('app.organization_id', true) as org,
               current_setting('app.job_worker', true) as worker`.execute(trx);
      return rows[0];
    });
    expect(leaked?.org === null || leaked?.org === '').toBe(true);
    expect(leaked?.worker === null || leaked?.worker === '').toBe(true);
  });
});
