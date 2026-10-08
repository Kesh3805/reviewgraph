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

  it('has no snapshots before the graph storage tables exist', async () => {
    expect(await scope.defaultSnapshot()).toBeNull();
    expect(await scope.snapshotBelongsTo()).toBe(false);
  });
});
