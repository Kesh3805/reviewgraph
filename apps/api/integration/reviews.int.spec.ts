import type { NestExpressApplication } from '@nestjs/platform-express';
import { counterTotal } from '../src/common/metrics';
import { as, createApiApp } from './api-app';
import {
  addMember,
  adminDb,
  cleanup,
  seedOrg,
  seedPullRequest,
  seedReviewerRun,
  seedReviewJob,
  seedReviewRun,
  seedUser,
  sha,
  type SeededOrg,
} from './seed';

describe('reviews API (integration)', () => {
  const admin = adminDb();
  let app: NestExpressApplication;
  let org: SeededOrg;
  let other: SeededOrg;
  let member: string;
  let viewer: string;

  beforeAll(async () => {
    [org, other] = await Promise.all([seedOrg(admin, 1), seedOrg(admin, 1)]);
    [member, viewer] = await Promise.all([seedUser(admin), seedUser(admin)]);
    await addMember(admin, org.organizationId, member, 'member');
    await addMember(admin, org.organizationId, viewer, 'viewer');
    app = await createApiApp();
  });

  afterAll(async () => {
    await app.close();
    await cleanup(admin, [org.organizationId, other.organizationId], [member, viewer]);
    await admin.destroy();
  });

  const run = (id: string) =>
    admin.selectFrom('review_runs').selectAll().where('id', '=', id).executeTakeFirstOrThrow();
  const job = (id: string) =>
    admin.selectFrom('jobs').selectAll().where('id', '=', id).executeTakeFirstOrThrow();

  it('manual_review_supersedes_running_run', async () => {
    const pr = await seedPullRequest(admin, org);
    // The head moved on since this run started (its webhook is still in flight).
    const running = await seedReviewRun(admin, org, pr, { state: 'REVIEWING', headSha: sha('c') });
    const queued = await seedReviewJob(admin, org, running);
    const before = counterTotal('manual_reviews_total');

    const res = await as(app, member).post(`/pull-requests/${pr}/review`).expect(202);
    expect(res.body).toMatchObject({
      created: true,
      head_sha: sha('a'),
      superseded_run_ids: [running],
    });
    const newId = res.body.review_run_id as string;

    expect(await run(running)).toMatchObject({ state: 'SUPERSEDED', superseded_by: newId });
    expect((await job(queued)).state).toBe('cancelled');
    expect(await run(newId)).toMatchObject({
      state: 'RECEIVED',
      trigger: 'manual',
      head_sha: sha('a'),
      retry_of: null,
    });
    const newJob = await job(res.body.job_id as string);
    expect(newJob).toMatchObject({ queue: 'pr-review', state: 'queued' });
    expect(newJob.payload).toEqual({ review_run_id: newId });
    expect(newJob.idempotency_key).toMatch(/^pr-review:github:\d+:\d+:a{40}:manual:\d{12}$/);
    expect(counterTotal('manual_reviews_total')).toBe(before + 1);

    // A double click within the minute is the same review.
    const again = await as(app, member).post(`/pull-requests/${pr}/review`).expect(202);
    expect(again.body).toMatchObject({ created: false, review_run_id: newId });

    const audit = await admin
      .selectFrom('audit_log')
      .select(['action', 'actor_id', 'target_id'])
      .where('organization_id', '=', org.organizationId)
      .where('action', '=', 'review.manual_triggered')
      .execute();
    expect(audit).toEqual([{ action: 'review.manual_triggered', actor_id: member, target_id: pr }]);
  });

  it('manual review of a closed pull request is 409, and viewers may not trigger', async () => {
    const closed = await seedPullRequest(admin, org, { state: 'closed' });
    await as(app, member).post(`/pull-requests/${closed}/review`).expect(409);
    const open = await seedPullRequest(admin, org);
    await as(app, viewer).post(`/pull-requests/${open}/review`).expect(403);
  });

  it('cancel_running_run_cancels_jobs', async () => {
    const pr = await seedPullRequest(admin, org);
    const active = await seedReviewRun(admin, org, pr, { state: 'ANALYZING' });
    const queued = await seedReviewJob(admin, org, active);
    const res = await as(app, member).post(`/reviews/${active}/cancel`).expect(200);
    expect(res.body).toEqual({ review_run_id: active, state: 'CANCELLED', cancelled_jobs: 1 });
    expect((await run(active)).state).toBe('CANCELLED');
    expect((await run(active)).completed_at).not.toBeNull();
    expect((await job(queued)).state).toBe('cancelled');
    // Idempotent.
    await as(app, member).post(`/reviews/${active}/cancel`).expect(200);
  });

  it('cancel_terminal_409', async () => {
    const pr = await seedPullRequest(admin, org);
    const done = await seedReviewRun(admin, org, pr, { state: 'COMPLETED' });
    const res = await as(app, member).post(`/reviews/${done}/cancel`).expect(409);
    expect(res.body).toMatchObject({ status: 409, state: 'COMPLETED' });
    expect((await run(done)).state).toBe('COMPLETED');
  });

  it('review_detail_completeness_from_rows', async () => {
    const pr = await seedPullRequest(admin, org);
    const id = await seedReviewRun(admin, org, pr, {
      traceParent: '00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01',
    });
    await seedReviewerRun(admin, org, id, 'security', 'succeeded', { clusterKey: 'a'.repeat(32) });
    await seedReviewerRun(admin, org, id, 'security', 'succeeded', { clusterKey: 'b'.repeat(32) });
    await seedReviewerRun(admin, org, id, 'correctness', 'failed', { errorClass: 'rate_limited' });
    await seedReviewerRun(admin, org, id, 'test', 'skipped');

    const res = await as(app, viewer).get(`/pull-requests/${pr}/reviews/${id}`).expect(200);
    expect(res.body.completeness).toEqual({
      reviewers_planned: 3,
      reviewers_succeeded: 1,
      reviewers_failed: [{ reviewer: 'correctness', error_class: 'rate_limited' }],
      not_executed: ['test'],
    });
    expect(res.body.reviewer_runs).toHaveLength(4);
    expect(res.body.reviewer_runs[0]).toMatchObject({ prompt_version: expect.any(String) });
    expect(res.body.trace_id).toBe('0af7651916cd43dd8448eb211c80319c');
    // The review must belong to the pull request in the path.
    const otherPr = await seedPullRequest(admin, org);
    await as(app, viewer).get(`/pull-requests/${otherPr}/reviews/${id}`).expect(404);
  });

  it('degraded_reason_listed', async () => {
    const pr = await seedPullRequest(admin, org);
    const id = await seedReviewRun(admin, org, pr, { degradedReviewers: ['performance'] });
    await seedReviewerRun(admin, org, id, 'performance', 'timed_out', { errorClass: 'transient' });
    await seedReviewerRun(admin, org, id, 'security', 'succeeded');
    const res = await as(app, viewer).get(`/pull-requests/${pr}/reviews/${id}`).expect(200);
    expect(res.body.degraded).toEqual({
      degraded: true,
      reviewers: ['performance'],
      reasons: [{ reviewer: 'performance', error_class: 'transient' }],
    });
  });

  it('pagination_cursor_stable', async () => {
    const pr = await seedPullRequest(admin, org);
    // Five finished runs; three share one created_at so the id breaks the tie.
    const t0 = new Date('2026-10-01T10:00:00.123Z');
    const ids: string[] = [];
    for (let i = 0; i < 5; i++) {
      ids.push(
        await seedReviewRun(admin, org, pr, {
          headSha: sha(String(i)),
          createdAt: i < 3 ? t0 : new Date(t0.getTime() + i * 1000),
        }),
      );
    }
    const pages: string[][] = [];
    let cursor: string | null = null;
    do {
      const query: string = cursor ? `?limit=2&cursor=${cursor}` : '?limit=2';
      const res = await as(app, viewer).get(`/pull-requests/${pr}/reviews${query}`).expect(200);
      pages.push((res.body.items as { id: string }[]).map((r) => r.id));
      cursor = res.body.next_cursor as string | null;
      // A newer run arriving between pages does not shift the pages already handed out.
      if (pages.length === 1) await seedReviewRun(admin, org, pr, { headSha: sha('f') });
    } while (cursor);
    const seen = pages.flat();
    expect(new Set(seen).size).toBe(seen.length);
    expect(seen).toHaveLength(5);
    expect(new Set(seen)).toEqual(new Set(ids));
    expect(pages.map((p) => p.length)).toEqual([2, 2, 1]);
    await as(app, viewer).get(`/pull-requests/${pr}/reviews?cursor=garbage`).expect(400);
  });

  it('lists pull requests by state with the latest review', async () => {
    const open = await seedPullRequest(admin, org);
    const latest = await seedReviewRun(admin, org, open);
    const res = await as(app, viewer)
      .get(`/repositories/${org.repositoryIds[0]}/pull-requests?state=open&limit=100`)
      .expect(200);
    const item = (res.body.items as { id: string; latest_review: { id: string } | null }[]).find(
      (p) => p.id === open,
    );
    expect(item?.latest_review?.id).toBe(latest);
    const closed = await as(app, viewer)
      .get(`/repositories/${org.repositoryIds[0]}/pull-requests?state=closed&limit=100`)
      .expect(200);
    expect((closed.body.items as { state: string }[]).every((p) => p.state !== 'open')).toBe(true);
    await as(app, viewer).get(`/pull-requests/${open}`).expect(200);
  });

  it('foreign pull requests and reviews are 404', async () => {
    const pr = await seedPullRequest(admin, other);
    const id = await seedReviewRun(admin, other, pr);
    await as(app, member).get(`/pull-requests/${pr}`).expect(404);
    await as(app, member).post(`/pull-requests/${pr}/review`).expect(404);
    await as(app, member).post(`/reviews/${id}/cancel`).expect(404);
    await as(app, member).get(`/repositories/${other.repositoryIds[0]}/pull-requests`).expect(404);
  });
});
