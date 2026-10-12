import { randomBytes, randomUUID } from 'node:crypto';
import { AuditService } from '../src/audit/audit.service';
import type { AppConfig } from '../src/config/config.module';
import { DbService } from '../src/db/db.module';
import { createKysely, createPool } from '../src/db/kysely.provider';
import { PgGraphScope } from '../src/graph/graph-scope';
import {
  adminDb,
  cleanup,
  seedOrg,
  seedPullRequest,
  seedReviewRun,
  seedUser,
  type SeededOrg,
} from './seed';

describe('PgGraphScope (integration)', () => {
  const admin = adminDb();
  const db = createKysely(
    createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max: 2 }),
  );
  const scope = new PgGraphScope(
    new DbService(db, { DB_APP_ROLE: 'rg_api' } as AppConfig),
    new AuditService(new DbService(db, { DB_APP_ROLE: 'rg_api' } as AppConfig)),
  );
  let org: SeededOrg;
  let other: SeededOrg;
  let user: string;

  beforeAll(async () => {
    [org, other] = await Promise.all([seedOrg(admin, 1), seedOrg(admin, 1)]);
    user = await seedUser(admin);
  });

  afterAll(async () => {
    await cleanup(admin, [org.organizationId, other.organizationId], [user]);
    await admin.destroy();
    await db.destroy();
  });

  it('resolves the repository of a review run within the tenant only', async () => {
    const pr = await seedPullRequest(admin, org);
    const run = await seedReviewRun(admin, org, pr);
    expect(await scope.reviewRepository(org.organizationId, run)).toBe(org.repositoryIds[0]);
    expect(await scope.reviewRepository(other.organizationId, run)).toBeNull();
  });

  it('audits source excerpt access', async () => {
    await scope.recordSourceAccess({
      organizationId: org.organizationId,
      repositoryId: org.repositoryIds[0]!,
      userId: user,
      snapshotId: '0190f3a2-0000-7000-8000-0000000000aa',
      path: 'src/config.ts',
      start: 1,
      end: 20,
      requestId: 'req-1',
    });
    const rows = await admin
      .selectFrom('audit_log')
      .select(['action', 'actor_id', 'target_id', 'metadata', 'request_id'])
      .where('organization_id', '=', org.organizationId)
      .where('action', '=', 'source.excerpt.read')
      .execute();
    expect(rows).toEqual([
      {
        action: 'source.excerpt.read',
        actor_id: user,
        target_id: 'src/config.ts',
        request_id: 'req-1',
        metadata: {
          snapshot_id: '0190f3a2-0000-7000-8000-0000000000aa',
          path: 'src/config.ts',
          start: 1,
          end: 20,
        },
      },
    ]);
  });

  it('resolves the latest ready full default-branch snapshot and validates ownership', async () => {
    const repo = org.repositoryIds[0]!;
    expect(await scope.defaultSnapshot(org.organizationId, repo)).toBeNull();
    const snapshot = async (
      target: SeededOrg,
      fields: { status?: string; purpose?: string; completedAt?: Date | null },
    ) => {
      const id = randomUUID();
      await admin
        .insertInto('snapshots')
        .values({
          id,
          organization_id: target.organizationId,
          repository_id: target.repositoryIds[0]!,
          commit_sha: 'a'.repeat(40),
          kind: 'full',
          purpose: fields.purpose ?? 'default_branch',
          status: fields.status ?? 'ready',
          graph_schema_version: 1,
          analyzer_versions: JSON.stringify({}),
          config_hash: Buffer.alloc(32, 1),
          fingerprint: randomBytes(32),
          completed_at: fields.completedAt ?? null,
        })
        .execute();
      return id;
    };
    const older = await snapshot(org, { completedAt: new Date('2026-10-01T00:00:00Z') });
    const latest = await snapshot(org, { completedAt: new Date('2026-10-02T00:00:00Z') });
    const indexing = await snapshot(org, { status: 'indexing' });
    await snapshot(org, { purpose: 'pull_request', completedAt: new Date('2026-10-03T00:00:00Z') });
    const foreign = await snapshot(other, { completedAt: new Date('2026-10-04T00:00:00Z') });

    expect(await scope.defaultSnapshot(org.organizationId, repo)).toBe(latest);
    expect(await scope.snapshotBelongsTo(org.organizationId, repo, older)).toBe(true);
    expect(await scope.snapshotBelongsTo(org.organizationId, repo, indexing)).toBe(false);
    expect(await scope.snapshotBelongsTo(org.organizationId, repo, foreign)).toBe(false);
    // Even naming the other tenant's repository, RLS hides its snapshot.
    expect(
      await scope.snapshotBelongsTo(org.organizationId, other.repositoryIds[0]!, foreign),
    ).toBe(false);
  });
});
