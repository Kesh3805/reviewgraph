import { randomUUID } from 'node:crypto';
import type { NestExpressApplication } from '@nestjs/platform-express';
import request from 'supertest';
import { counterTotalAll } from '../../src/common/metrics';
import { asUser, CSRF_HEADERS } from '../../test/helpers/tenancy-probe';
import { fillPath, ISOLATION_MATRIX, TENANT_ID_PARAMS } from '../../test/security/isolation-matrix';
import { as, createApiApp } from '../api-app';
import { adminDb, cleanup } from '../seed';
import { buildTenant, type Tenant } from './tenancy.fixture';

/** Bodies that would pass validation, so a 404 can only come from the tenancy check. */
const BODIES: Record<string, (t: Tenant) => object> = {
  'POST /api/v1/repositories': (t) => ({
    installation_id: t.installationId,
    full_name: 'acme/repo-0',
  }),
  'PATCH /api/v1/repositories/{repoId}/settings': () => ({ skip_bots: false }),
  'POST /api/v1/findings/{findingId}/feedback': () => ({ verdict: 'false_positive' }),
  'POST /api/v1/repositories/{repoId}/graph/subgraph': () => ({ seeds: ['k1'] }),
};

const QUERIES: Record<string, string> = {
  'GET /api/v1/repositories/{repoId}/graph/path': '?from=a&to=b',
  'GET /api/v1/repositories/{repoId}/source': '?path=src/a.ts&start=1&end=2',
};

const FOREIGN_ROUTES = Object.entries(ISOLATION_MATRIX)
  .filter(([, behavior]) => behavior === 'foreign_id_404')
  .map(([route]) => route);

describe('route isolation across two organizations (SEC-001)', () => {
  const admin = adminDb();
  let app: NestExpressApplication;
  let a: Tenant;
  let b: Tenant;

  beforeAll(async () => {
    [a, b] = await Promise.all([buildTenant(admin), buildTenant(admin)]);
    app = await createApiApp();
  });

  afterAll(async () => {
    await app.close();
    await cleanup(admin, [a.org.organizationId, b.org.organizationId], [a.owner, b.owner]);
    await admin.destroy();
  });

  /** Calls a matrix route as `user` with the given ids. */
  const call = (route: string, user: string, ids: Tenant['ids'], tenant: Tenant) => {
    const [method, path] = route.split(' ') as [string, string];
    const url = `${fillPath(path, ids)}${QUERIES[route] ?? ''}`;
    const agent = request(app.getHttpServer());
    const req =
      method === 'GET'
        ? agent.get(url)
        : method === 'POST'
          ? agent.post(url)
          : method === 'PATCH'
            ? agent.patch(url)
            : agent.delete(url);
    return req
      .set('cookie', asUser(user))
      .set(CSRF_HEADERS)
      .send(BODIES[route]?.(tenant) ?? {});
  };

  /** The problem body without the per-request members. */
  const stable = (body: Record<string, unknown>) =>
    Object.fromEntries(
      Object.entries(body).filter(([k]) => k !== 'instance' && k !== 'request_id'),
    );

  it('foreign_repo_id_returns_404_on_all_repo_routes', async () => {
    const repoRoutes = FOREIGN_ROUTES.filter((r) => r.includes('{repoId}'));
    expect(repoRoutes.length).toBeGreaterThan(10);
    const denied = counterTotalAll('tenancy_denied_total');
    for (const route of repoRoutes) {
      const res = await call(route, a.owner, { ...a.ids, repoId: b.ids.repoId }, a);
      expect([route, res.status]).toEqual([route, 404]);
    }
    expect(counterTotalAll('tenancy_denied_total')).toBeGreaterThanOrEqual(
      denied + repoRoutes.length,
    );
  });

  it('foreign_review_finding_pr_ids_return_404', async () => {
    for (const route of FOREIGN_ROUTES) {
      for (const param of TENANT_ID_PARAMS) {
        if (!route.includes(`{${param}}`)) continue;
        // Only the one param points at the other organization.
        const res = await call(route, a.owner, { ...a.ids, [param]: b.ids[param] }, a);
        expect([route, param, res.status]).toEqual([route, param, 404]);
      }
    }
    // A body-borne foreign id (installation) is a 404 too.
    const res = await call('POST /api/v1/repositories', a.owner, a.ids, b);
    expect(res.status).toBe(404);
  });

  it('existence_oracle_absent_identical_404_bodies', async () => {
    for (const route of FOREIGN_ROUTES) {
      for (const param of TENANT_ID_PARAMS) {
        if (!route.includes(`{${param}}`)) continue;
        const foreign = await call(route, a.owner, { ...a.ids, [param]: b.ids[param] }, a);
        const unknown = await call(route, a.owner, { ...a.ids, [param]: randomUUID() }, a);
        expect([route, param, foreign.status]).toEqual([route, param, unknown.status]);
        expect(stable(foreign.body as Record<string, unknown>)).toEqual(
          stable(unknown.body as Record<string, unknown>),
        );
        expect(foreign.headers['content-type']).toBe(unknown.headers['content-type']);
        expect(foreign.headers['retry-after']).toBe(unknown.headers['retry-after']);
      }
    }
  });

  it('list_endpoints_never_return_other_org_rows', async () => {
    const repos = await as(app, a.owner).get('/repositories?limit=100').expect(200);
    const repoIds = (repos.body.items as { id: string; organization_id: string }[]).map(
      (r) => r.id,
    );
    expect(repoIds).toContain(a.ids.repoId);
    expect(repoIds).not.toContain(b.ids.repoId);
    await as(app, a.owner).get(`/repositories?organization_id=${b.org.organizationId}`).expect(404);

    const prs = await as(app, a.owner)
      .get(`/repositories/${a.ids.repoId}/pull-requests?state=all`)
      .expect(200);
    expect((prs.body.items as { id: string }[]).map((p) => p.id)).toEqual([a.pullRequestId]);
    const findings = await as(app, a.owner).get(`/reviews/${a.reviewRunId}/findings`).expect(200);
    expect((findings.body.items as { id: string }[]).map((f) => f.id)).toEqual([a.findingId]);
    const feedback = await as(app, a.owner).get(`/findings/${a.findingId}/feedback`).expect(200);
    expect((feedback.body.items as { user_id: string }[]).every((f) => f.user_id === a.owner)).toBe(
      true,
    );
    const audit = await as(app, a.owner)
      .get(`/organizations/${a.org.organizationId}/audit?limit=200`)
      .expect(200);
    const auditText = JSON.stringify(audit.body);
    for (const id of Object.values(b.ids)) expect(auditText).not.toContain(id);
  });

  it('foreign_cursor_cannot_page_other_org', async () => {
    // Org B's owner pages B's reviews and hands the cursor to org A's owner.
    for (let i = 0; i < 2; i++) {
      await admin
        .insertInto('review_runs')
        .values({
          organization_id: b.org.organizationId,
          repository_id: b.ids.repoId,
          pull_request_id: b.pullRequestId,
          base_sha: 'b'.repeat(40),
          head_sha: String(i).repeat(40),
          state: 'COMPLETED',
          trigger: 'webhook',
        })
        .execute();
    }
    const page = await as(app, b.owner)
      .get(`/pull-requests/${b.pullRequestId}/reviews?limit=1`)
      .expect(200);
    const cursor = page.body.next_cursor as string;
    expect(cursor).toEqual(expect.any(String));
    await as(app, a.owner)
      .get(`/pull-requests/${b.pullRequestId}/reviews?limit=1&cursor=${cursor}`)
      .expect(404);
    const mine = await as(app, a.owner)
      .get(`/pull-requests/${a.pullRequestId}/reviews?limit=100&cursor=${cursor}`)
      .expect(200);
    const ids = (mine.body.items as { pull_request_id: string }[]).map((r) => r.pull_request_id);
    expect(ids.every((id) => id === a.pullRequestId)).toBe(true);
  });

  it('mutations_on_foreign_ids_do_not_change_data', async () => {
    const settingsBefore = await admin
      .selectFrom('repository_settings')
      .selectAll()
      .where('repository_id', '=', b.ids.repoId)
      .executeTakeFirstOrThrow();
    await as(app, a.owner)
      .patch(`/repositories/${b.ids.repoId}/settings`, { skip_bots: false })
      .expect(404);
    await as(app, a.owner).post(`/reviews/${b.reviewRunId}/cancel`).expect(404);
    await as(app, a.owner)
      .post(`/findings/${b.findingId}/feedback`, { verdict: 'false_positive' })
      .expect(404);
    await as(app, a.owner).post(`/pull-requests/${b.pullRequestId}/review`).expect(404);

    const settingsAfter = await admin
      .selectFrom('repository_settings')
      .selectAll()
      .where('repository_id', '=', b.ids.repoId)
      .executeTakeFirstOrThrow();
    expect(settingsAfter).toEqual(settingsBefore);
    const run = await admin
      .selectFrom('review_runs')
      .select('state')
      .where('id', '=', b.reviewRunId)
      .executeTakeFirstOrThrow();
    expect(run.state).toBe('REVIEWING');
    const feedback = await admin
      .selectFrom('feedback')
      .select(['user_id', 'verdict'])
      .where('finding_id', '=', b.findingId)
      .execute();
    expect(feedback).toEqual([{ user_id: b.owner, verdict: 'useful' }]);
    const runs = await admin
      .selectFrom('review_runs')
      .select('id')
      .where('pull_request_id', '=', b.pullRequestId)
      .where('trigger', '=', 'manual')
      .execute();
    expect(runs).toEqual([]);
  });
});
