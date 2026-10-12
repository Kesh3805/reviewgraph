import { randomUUID } from 'node:crypto';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { counterTotal } from '../src/common/metrics';
import { SUPPRESSION_WRITER, type NewSuppression } from '../src/findings/suppression.port';
import { as, createApiApp } from './api-app';
import {
  addMember,
  adminDb,
  cleanup,
  seedFinding,
  seedOrg,
  seedPullRequest,
  seedReviewerRun,
  seedReviewRun,
  seedUser,
  type SeededFinding,
  type SeededOrg,
} from './seed';

describe('feedback API (integration)', () => {
  const admin = adminDb();
  const created: NewSuppression[] = [];
  let app: NestExpressApplication;
  let org: SeededOrg;
  let other: SeededOrg;
  let viewer: string;
  let member: string;
  let security: SeededFinding;
  let correctness: SeededFinding;
  let suppressed: SeededFinding;

  beforeAll(async () => {
    [org, other] = await Promise.all([seedOrg(admin, 1), seedOrg(admin, 1)]);
    [viewer, member] = await Promise.all([seedUser(admin), seedUser(admin)]);
    await addMember(admin, org.organizationId, viewer, 'viewer');
    await addMember(admin, org.organizationId, member, 'member');
    const pr = await seedPullRequest(admin, org);
    const run = await seedReviewRun(admin, org, pr);
    const sec = await seedReviewerRun(admin, org, run, 'security', 'succeeded');
    const cor = await seedReviewerRun(admin, org, run, 'correctness', 'succeeded');
    security = await seedFinding(admin, org, run, sec, { published: true });
    correctness = await seedFinding(admin, org, run, cor, {
      reviewer: 'correctness',
      published: true,
    });
    suppressed = await seedFinding(admin, org, run, cor, {
      reviewer: 'correctness',
      state: 'SUPPRESSED_PREEXISTING',
      suppression: { reason: { type: 'preexisting' }, detail: 'on base', stage: 5 },
    });
    app = await createApiApp((builder) =>
      builder.overrideProvider(SUPPRESSION_WRITER).useValue({
        create: (_trx: unknown, s: NewSuppression) => {
          created.push(s);
          return Promise.resolve(randomUUID());
        },
      }),
    );
  });

  afterAll(async () => {
    await app.close();
    await cleanup(admin, [org.organizationId, other.organizationId], [viewer, member]);
    await admin.destroy();
  });

  const feedbackRows = (findingId: string) =>
    admin.selectFrom('feedback').selectAll().where('finding_id', '=', findingId).execute();

  it('feedback_upsert_latest_wins', async () => {
    const id = security.verifiedId!;
    const before = counterTotal('finding_feedback_total', {
      verdict: 'false_positive',
      reviewer: 'security',
    });
    const first = await as(app, viewer)
      .post(`/findings/${id}/feedback`, { verdict: 'useful' })
      .expect(201);
    expect(first.body.feedback).toMatchObject({
      verdict: 'useful',
      source: 'web',
      user_id: viewer,
    });
    const second = await as(app, viewer)
      .post(`/findings/${id}/feedback`, { verdict: 'false_positive', comment: 'not reachable' })
      .expect(201);
    expect(second.body.feedback.id).toBe(first.body.feedback.id);
    const rows = await feedbackRows(id);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({ verdict: 'false_positive', comment: 'not reachable' });
    expect(
      counterTotal('finding_feedback_total', { verdict: 'false_positive', reviewer: 'security' }),
    ).toBe(before + 1);

    const list = await as(app, viewer).get(`/findings/${id}/feedback`).expect(200);
    expect(list.body.items).toHaveLength(1);
  });

  it('feedback_audited', async () => {
    const id = correctness.verifiedId!;
    await as(app, member).post(`/findings/${id}/feedback`, { verdict: 'useful' }).expect(201);
    await as(app, member)
      .post(`/findings/${id}/feedback`, { verdict: 'already_handled' })
      .expect(201);
    const audit = await admin
      .selectFrom('audit_log')
      .select(['action', 'actor_id', 'metadata'])
      .where('organization_id', '=', org.organizationId)
      .where('target_id', '=', id)
      .orderBy('occurred_at')
      .execute();
    expect(audit.map((a) => a.action)).toEqual([
      'finding.feedback.created',
      'finding.feedback.updated',
    ]);
    expect(audit[1]!.metadata).toMatchObject({
      verdict: 'already_handled',
      previous_verdict: 'useful',
    });
    expect(audit.every((a) => a.actor_id === member)).toBe(true);
  });

  it('suppression_requires_maintainer', async () => {
    const res = await as(app, viewer)
      .post(`/findings/${security.verifiedId}/feedback`, {
        verdict: 'intentional',
        create_suppression: { kind: 'fingerprint', reason: 'by design' },
      })
      .expect(403);
    expect(res.body.status).toBe(403);
    expect(created).toHaveLength(0);

    const ok = await as(app, member)
      .post(`/findings/${security.verifiedId}/feedback`, {
        verdict: 'intentional',
        create_suppression: { kind: 'path', reason: 'generated code' },
      })
      .expect(201);
    expect(ok.body.suppression_id).toEqual(expect.any(String));
    expect(created).toEqual([
      expect.objectContaining({
        kind: 'path',
        value: 'src/users/user.controller.ts',
        reason: 'generated code',
        createdBy: member,
        organizationId: org.organizationId,
      }),
    ]);
    const audit = await admin
      .selectFrom('audit_log')
      .select('action')
      .where('organization_id', '=', org.organizationId)
      .where('action', '=', 'suppression.created')
      .execute();
    expect(audit).toHaveLength(1);
  });

  it('suppression_only_for_intentional_or_not_relevant', async () => {
    const res = await as(app, member)
      .post(`/findings/${security.verifiedId}/feedback`, {
        verdict: 'useful',
        create_suppression: { kind: 'fingerprint', reason: 'x' },
      })
      .expect(422);
    expect(res.body.status).toBe(422);
    await as(app, member)
      .post(`/findings/${security.verifiedId}/feedback`, { verdict: 'meh' })
      .expect(422);
    await as(app, member)
      .post(`/findings/${security.verifiedId}/feedback`, {
        verdict: 'useful',
        comment: 'x'.repeat(2001),
      })
      .expect(422);
  });

  it('feedback_summary_rates', async () => {
    // Fresh repository data: two more users on the security finding.
    const [u1, u2] = await Promise.all([seedUser(admin), seedUser(admin)]);
    await addMember(admin, org.organizationId, u1, 'viewer');
    await addMember(admin, org.organizationId, u2, 'viewer');
    try {
      await as(app, u1)
        .post(`/findings/${security.verifiedId}/feedback`, { verdict: 'useful' })
        .expect(201);
      await as(app, u2)
        .post(`/findings/${security.verifiedId}/feedback`, { verdict: 'useful' })
        .expect(201);
      const res = await as(app, viewer)
        .get(`/repositories/${org.repositoryIds[0]}/feedback/summary`)
        .expect(200);
      const rows = await admin
        .selectFrom('feedback')
        .select('verdict')
        .where('organization_id', '=', org.organizationId)
        .execute();
      const useful = rows.filter((r) => r.verdict === 'useful').length;
      const fp = rows.filter((r) => r.verdict === 'false_positive').length;
      expect(res.body.total).toBe(rows.length);
      expect(res.body.acceptance_rate).toBeCloseTo(useful / rows.length);
      expect(res.body.false_positive_rate).toBeCloseTo(fp / rows.length);
      expect(res.body.by_reviewer.security.total).toBeGreaterThan(0);
      const future = await as(app, viewer)
        .get(
          `/repositories/${org.repositoryIds[0]}/feedback/summary?since=${encodeURIComponent('2999-01-01T00:00:00Z')}`,
        )
        .expect(200);
      expect(future.body).toMatchObject({ total: 0, acceptance_rate: null });
    } finally {
      await cleanup(admin, [], [u1, u2]);
    }
  });

  it('feedback on an unverified finding is 409, on a foreign one 404', async () => {
    await as(app, member)
      .post(`/findings/${suppressed.candidateId}/feedback`, { verdict: 'useful' })
      .expect(409);
    const pr = await seedPullRequest(admin, other);
    const run = await seedReviewRun(admin, other, pr);
    const rr = await seedReviewerRun(admin, other, run, 'security', 'succeeded');
    const foreign = await seedFinding(admin, other, run, rr, { published: true });
    await as(app, member)
      .post(`/findings/${foreign.verifiedId}/feedback`, { verdict: 'useful' })
      .expect(404);
    await as(app, member)
      .get(`/repositories/${other.repositoryIds[0]}/feedback/summary`)
      .expect(404);
  });
});
