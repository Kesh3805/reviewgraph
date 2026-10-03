import type { NestExpressApplication } from '@nestjs/platform-express';
import { sql } from 'kysely';
import request from 'supertest';
import { createKysely, createPool } from '../src/db/kysely.provider';
import { runInTx } from '../src/db/tx';
import { createTestApp } from '../test/helpers';
import { installTestAuth, TenancyProbeController } from '../test/helpers/tenancy-probe';
import { addMember, adminDb, cleanup, seedOrg, seedUser, type SeededOrg } from './seed';

const ROLE = { role: 'rg_api' };
const TENANT_TABLES = [
  'organizations',
  'provider_installations',
  'memberships',
  'repositories',
  'pull_requests',
  'review_runs',
  'reviewer_runs',
  'candidate_findings',
  'verified_findings',
  'published_findings',
  'finding_feedback',
  'webhook_deliveries',
];

describe('row level security (integration)', () => {
  const admin = adminDb();
  // A single connection: every transaction below reuses it, which is what proves the isolation.
  const appDb = createKysely(
    createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max: 1 }),
  );
  let a: SeededOrg;
  let b: SeededOrg;

  beforeAll(async () => {
    a = await seedOrg(admin, 2);
    b = await seedOrg(admin, 3);
  });

  afterAll(async () => {
    await cleanup(admin, [a.organizationId, b.organizationId]);
    await admin.destroy();
    await appDb.destroy();
  });

  const repoIds = (orgId: string | null) =>
    runInTx(
      appDb,
      orgId,
      async (trx) => (await trx.selectFrom('repositories').select('id').execute()).map((r) => r.id),
      ROLE,
    );

  it('rls_hides_other_org_rows', async () => {
    expect((await repoIds(a.organizationId)).sort()).toEqual([...a.repositoryIds].sort());
    expect((await repoIds(b.organizationId)).sort()).toEqual([...b.repositoryIds].sort());
    const orgs = await runInTx(
      appDb,
      a.organizationId,
      (trx) => trx.selectFrom('organizations').select('id').execute(),
      ROLE,
    );
    expect(orgs.map((o) => o.id)).toEqual([a.organizationId]);
  });

  it('rls_without_setting_returns_nothing', async () => {
    expect(await repoIds(null)).toEqual([]);
    // No transaction helper at all: the role alone, with no setting ever made on this connection.
    const bare = await appDb.transaction().execute(async (trx) => {
      await sql`set local role rg_api`.execute(trx);
      return trx.selectFrom('repositories').select('id').execute();
    });
    expect(bare).toEqual([]);
  });

  it('rls_with_check_blocks_cross_org_insert', async () => {
    await expect(
      runInTx(
        appDb,
        a.organizationId,
        (trx) =>
          trx
            .insertInto('repositories')
            .values({
              organization_id: b.organizationId,
              installation_id: b.installationId,
              provider: 'github',
              provider_repo_id: 'cross-org',
              full_name: 'x/y',
              default_branch: 'main',
              visibility: 'private',
            })
            .execute(),
        ROLE,
      ),
    ).rejects.toThrow(/row-level security/);
  });

  it('cannot update or delete rows of another organization', async () => {
    const updated = await runInTx(
      appDb,
      a.organizationId,
      (trx) =>
        trx
          .updateTable('repositories')
          .set({ default_branch: 'evil' })
          .where('id', 'in', b.repositoryIds)
          .executeTakeFirst(),
      ROLE,
    );
    expect(updated.numUpdatedRows).toBe(0n);
    const deleted = await runInTx(
      appDb,
      a.organizationId,
      (trx) => trx.deleteFrom('repositories').where('id', 'in', b.repositoryIds).executeTakeFirst(),
      ROLE,
    );
    expect(deleted.numDeletedRows).toBe(0n);
  });

  it('setting_does_not_leak_across_pooled_connections', async () => {
    expect((await repoIds(a.organizationId)).length).toBe(2);
    // Same single pooled connection, no tenant: nothing may be visible.
    expect(await repoIds(null)).toEqual([]);
    const leaked = await appDb.transaction().execute(async (trx) => {
      await sql`set local role rg_api`.execute(trx);
      const { rows } = await sql<{
        v: string | null;
      }>`select current_setting('app.organization_id', true) as v`.execute(trx);
      return rows[0]?.v;
    });
    expect(leaked === null || leaked === '').toBe(true);
  });

  it('enables and forces RLS on every tenant table', async () => {
    const { rows } = await sql<{
      relname: string;
      relrowsecurity: boolean;
      relforcerowsecurity: boolean;
    }>`
      select relname, relrowsecurity, relforcerowsecurity from pg_class
      where relnamespace = 'public'::regnamespace and relname = any(${TENANT_TABLES})`.execute(
      admin,
    );
    expect(rows.map((r) => r.relname).sort()).toEqual([...TENANT_TABLES].sort());
    for (const row of rows) {
      expect([row.relname, row.relrowsecurity, row.relforcerowsecurity]).toEqual([
        row.relname,
        true,
        true,
      ]);
    }
  });

  it('webhook_deliveries rows without an organization are invisible to rg_api', async () => {
    const id = `rls-null-org-${Date.now()}`;
    await admin
      .insertInto('webhook_deliveries')
      .values({
        provider: 'github',
        delivery_id: id,
        event: 'ping',
        payload_sha256: 'a'.repeat(64),
        signature_valid: true,
        status: 'received',
      })
      .execute();
    try {
      const seen = await runInTx(
        appDb,
        a.organizationId,
        (trx) =>
          trx.selectFrom('webhook_deliveries').select('id').where('delivery_id', '=', id).execute(),
        ROLE,
      );
      expect(seen).toEqual([]);
    } finally {
      await admin.deleteFrom('webhook_deliveries').where('delivery_id', '=', id).execute();
    }
  });

  describe('tenancy guard against the real database', () => {
    let app: NestExpressApplication;
    let viewer: string;
    let member: string;
    let outsider: string;

    beforeAll(async () => {
      [viewer, member, outsider] = await Promise.all([
        seedUser(admin),
        seedUser(admin),
        seedUser(admin),
      ]);
      await addMember(admin, a.organizationId, viewer, 'viewer');
      await addMember(admin, a.organizationId, member, 'member');
      app = await createTestApp(undefined, [TenancyProbeController], {
        env: { DATABASE_URL: process.env.RG_TEST_DATABASE_URL!, DB_APP_ROLE: 'rg_api' },
      });
      installTestAuth(app);
      await app.init();
    });

    afterAll(async () => {
      await app.close();
      await cleanup(admin, [], [viewer, member, outsider]);
    });

    const call = (method: 'get' | 'patch', repo: string, user: string) =>
      request(app.getHttpServer())[method](`/api/v1/probe/repos/${repo}`).set('x-test-user', user);

    it('guard_404_for_foreign_repo', async () => {
      await call('get', b.repositoryIds[0]!, viewer).expect(404);
      await call('get', a.repositoryIds[0]!, outsider).expect(404);
      await call('get', a.repositoryIds[0]!, viewer).expect(200);
    });

    it('guard_role_maintainer_required', async () => {
      await call('patch', a.repositoryIds[0]!, viewer).expect(403);
      const res = await call('patch', a.repositoryIds[0]!, member).expect(200);
      expect(res.body).toEqual({ organizationId: a.organizationId, role: 'member' });
    });
  });
});
