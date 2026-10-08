import type { NestExpressApplication } from '@nestjs/platform-express';
import { sql } from 'kysely';
import { AuditService } from '../src/audit/audit.service';
import type { AppConfig } from '../src/config/config.module';
import { DbService } from '../src/db/db.module';
import { createKysely, createPool } from '../src/db/kysely.provider';
import { runInTx } from '../src/db/tx';
import { as, createApiApp } from './api-app';
import { addMember, adminDb, cleanup, seedOrg, seedUser, type SeededOrg } from './seed';

describe('audit log (integration)', () => {
  const admin = adminDb();
  const db = createKysely(
    createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max: 12 }),
  );
  const dbs = new DbService(db, { DB_APP_ROLE: 'rg_api' } as AppConfig);
  const audit = new AuditService(dbs);
  let app: NestExpressApplication;
  let org: SeededOrg;
  let other: SeededOrg;
  let orgAdmin: string;
  let member: string;

  beforeAll(async () => {
    [org, other] = await Promise.all([seedOrg(admin, 1), seedOrg(admin, 1)]);
    [orgAdmin, member] = await Promise.all([seedUser(admin), seedUser(admin)]);
    await addMember(admin, org.organizationId, orgAdmin, 'admin');
    await addMember(admin, org.organizationId, member, 'member');
    app = await createApiApp();
  });

  afterAll(async () => {
    await app.close();
    await cleanup(admin, [org.organizationId, other.organizationId], [orgAdmin, member]);
    await admin.destroy();
    await db.destroy();
  });

  const event = (orgId: string, action = 'repository.settings.updated') => ({
    organizationId: orgId,
    actor: { type: 'system' as const },
    action,
    targetType: 'repository',
  });

  const rowsOf = (orgId: string, action?: string) => {
    let q = admin.selectFrom('audit_log').selectAll().where('organization_id', '=', orgId);
    if (action) q = q.where('action', '=', action);
    return q.orderBy('chain_seq').execute();
  };

  it('config_change_writes_audit_row_in_same_transaction', async () => {
    const repo = org.repositoryIds[0]!;
    await as(app, member).patch(`/repositories/${repo}/settings`, { skip_bots: false }).expect(200);
    const rows = await rowsOf(org.organizationId, 'repository.settings.updated');
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ actor_id: member, target_id: repo });
    expect(rows[0]!.metadata).toMatchObject({ changed: { skip_bots: { from: true, to: false } } });
  });

  it('audit_failure_rolls_back_mutation', async () => {
    const repo = org.repositoryIds[0]!;
    await expect(
      dbs.withTx(org.organizationId, async (trx) => {
        await trx
          .updateTable('repository_settings')
          .set({ skip_drafts: false })
          .where('repository_id', '=', repo)
          .execute();
        // An action longer than the CHECK allows: the audit write fails.
        await audit.record(trx, event(org.organizationId, 'x'.repeat(101)));
      }),
    ).rejects.toThrow();
    const settings = await admin
      .selectFrom('repository_settings')
      .select('skip_drafts')
      .where('repository_id', '=', repo)
      .executeTakeFirstOrThrow();
    expect(settings.skip_drafts).toBe(true);
  });

  it('update_and_delete_on_audit_log_rejected_for_api_role', async () => {
    await dbs.withTx(org.organizationId, (trx) => audit.record(trx, event(org.organizationId)));
    await expect(
      runInTx(
        db,
        org.organizationId,
        (trx) =>
          trx
            .updateTable('audit_log')
            .set({ action: 'forged' })
            .where('organization_id', '=', org.organizationId)
            .execute(),
        { role: 'rg_api' },
      ),
    ).rejects.toThrow(/permission denied|append-only/);
    await expect(
      runInTx(
        db,
        org.organizationId,
        (trx) =>
          trx.deleteFrom('audit_log').where('organization_id', '=', org.organizationId).execute(),
        { role: 'rg_api' },
      ),
    ).rejects.toThrow(/permission denied|append-only/);
  });

  it('hash_chain_detects_row_tampering', async () => {
    const tampered = await seedOrg(admin, 1);
    try {
      for (let i = 0; i < 3; i++) {
        await dbs.withTx(tampered.organizationId, (trx) =>
          audit.record(trx, { ...event(tampered.organizationId), metadata: { i } }),
        );
      }
      expect(await audit.verify(tampered.organizationId)).toEqual({
        ok: true,
        checked: 3,
        first_invalid_id: null,
      });
      const rows = await rowsOf(tampered.organizationId);
      // A privileged user (the superuser test login bypasses the append-only trigger) edits a row.
      await admin
        .updateTable('audit_log')
        .set({ metadata: JSON.stringify({ i: 99 }) })
        .where('id', '=', rows[1]!.id)
        .execute();
      expect(await audit.verify(tampered.organizationId)).toMatchObject({
        ok: false,
        first_invalid_id: rows[1]!.id,
      });
    } finally {
      await cleanup(admin, [tampered.organizationId]);
    }
  });

  it('concurrent_inserts_keep_chain_gapless', async () => {
    const busy = await seedOrg(admin, 1);
    try {
      await Promise.all(
        Array.from({ length: 10 }, (_, i) =>
          dbs.withTx(busy.organizationId, (trx) =>
            audit.record(trx, { ...event(busy.organizationId), metadata: { i } }),
          ),
        ),
      );
      const rows = await rowsOf(busy.organizationId);
      expect(rows.map((r) => Number(r.chain_seq))).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
      for (let i = 1; i < rows.length; i++) {
        expect(rows[i]!.prev_hash).toEqual(rows[i - 1]!.hash);
      }
      expect((await audit.verify(busy.organizationId)).ok).toBe(true);
    } finally {
      await cleanup(admin, [busy.organizationId]);
    }
  });

  it('publication_and_feedback_are_audited_once_under_retry', async () => {
    const key = `publish:${org.repositoryIds[0]}:${'a'.repeat(40)}`;
    const write = () =>
      dbs.withTx(org.organizationId, (trx) =>
        audit.record(trx, { ...event(org.organizationId, 'review.published'), dedupeKey: key }),
      );
    expect(await write()).toBe(true);
    expect(await write()).toBe(false);
    expect(await rowsOf(org.organizationId, 'review.published')).toHaveLength(1);
    expect((await audit.verify(org.organizationId)).ok).toBe(true);
  });

  it('audit_read_requires_admin_and_is_tenant_scoped', async () => {
    await as(app, member).get(`/organizations/${org.organizationId}/audit`).expect(403);
    await as(app, orgAdmin).get(`/organizations/${other.organizationId}/audit`).expect(404);
    const res = await as(app, orgAdmin)
      .get(`/organizations/${org.organizationId}/audit?limit=2`)
      .expect(200);
    expect(res.body.items).toHaveLength(2);
    expect(res.body.next_cursor).toEqual(expect.any(String));
    const next = await as(app, orgAdmin)
      .get(`/organizations/${org.organizationId}/audit?limit=200&cursor=${res.body.next_cursor}`)
      .expect(200);
    const seen = [...res.body.items, ...next.body.items].map((i: { id: string }) => i.id);
    expect(new Set(seen).size).toBe(seen.length);
    expect(seen).toHaveLength((await rowsOf(org.organizationId)).length);
    const filtered = await as(app, orgAdmin)
      .get(`/organizations/${org.organizationId}/audit?action=review.published`)
      .expect(200);
    expect(filtered.body.items).toHaveLength(1);
    const verify = await as(app, orgAdmin)
      .get(`/organizations/${org.organizationId}/audit/verify`)
      .expect(200);
    expect(verify.body.ok).toBe(true);
  });

  it('metadata_never_contains_secret_patterns', async () => {
    await dbs.withTx(org.organizationId, (trx) =>
      audit.record(trx, {
        ...event(org.organizationId, 'credentials.issued'),
        metadata: { token: 'ghs_secretvalue', note: 'key AKIAABCDEFGHIJKLMNOP leaked' },
      }),
    );
    const [row] = await rowsOf(org.organizationId, 'credentials.issued');
    const text = JSON.stringify(row!.metadata);
    expect(text).not.toContain('ghs_secretvalue');
    expect(text).not.toContain('AKIAABCDEFGHIJKLMNOP');
  });

  it('denied_access_event_survives_request_rollback', async () => {
    await expect(
      dbs.withTx(org.organizationId, async () => {
        await audit.recordStandalone({
          ...event(org.organizationId, 'access.denied'),
          outcome: 'denied',
        });
        throw new Error('request failed');
      }),
    ).rejects.toThrow('request failed');
    expect(await rowsOf(org.organizationId, 'access.denied')).toHaveLength(1);
  });

  it('the chain trigger runs as rg_ops (works without seeing other tenants)', async () => {
    const { rows } = await sql<{ owner: string }>`
      select pg_get_userbyid(proowner) as owner from pg_proc where proname = 'rg_audit_chain'`.execute(
      admin,
    );
    expect(rows[0]?.owner).toBe('rg_ops');
  });
});
