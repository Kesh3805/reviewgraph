import type { NestExpressApplication } from '@nestjs/platform-express';
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

const sym = (display: string) => ({ id: `ts:${display}`, key: 'k'.repeat(32), display });

/** The PRD section 86 auth-bypass finding: a caller path that skips AuthService.authorize. */
const AUTH_BYPASS_EVIDENCE = [
  {
    kind: 'caller_path',
    claimed_strength: 'strong',
    origin: { type: 'reviewer', reviewer: 'security' },
    verification: { status: 'confirmed', stage: 3 },
    claim: 'update reaches updateUser without passing authorize',
    location: { path: 'src/users/user.controller.ts', side: 'head', lines: { start: 10, end: 14 } },
    symbols: [sym('UserController.update')],
    relation: {
      from: sym('UserController.update'),
      to: sym('AuthService.authorize'),
      relation: 'reaches',
      via: [sym('AdminService.updateUser')],
    },
    // Not part of the typed evidence: must never be returned.
    raw_output: 'MODEL-RAW-OUTPUT',
    prompt: 'SYSTEM-PROMPT',
  },
];

const STAGES = Array.from({ length: 8 }, (_, i) => ({
  stage: i + 1,
  outcome: 'pass',
  reason: null,
}));

describe('findings API (integration)', () => {
  const admin = adminDb();
  let app: NestExpressApplication;
  let org: SeededOrg;
  let other: SeededOrg;
  let viewer: string;
  let runId: string;
  let published: SeededFinding;
  let verifiedOnly: SeededFinding;
  let lowConfidence: SeededFinding;
  let duplicate: SeededFinding;

  beforeAll(async () => {
    [org, other] = await Promise.all([seedOrg(admin, 1), seedOrg(admin, 1)]);
    viewer = await seedUser(admin);
    await addMember(admin, org.organizationId, viewer, 'viewer');
    const pr = await seedPullRequest(admin, org);
    runId = await seedReviewRun(admin, org, pr);
    const security = await seedReviewerRun(admin, org, runId, 'security', 'succeeded');
    const correctness = await seedReviewerRun(admin, org, runId, 'correctness', 'succeeded');
    published = await seedFinding(admin, org, runId, security, {
      published: true,
      severity: 'critical',
      evidence: AUTH_BYPASS_EVIDENCE,
      verified: { severity: 'critical', confidence: 0.92, stageOutcomes: STAGES },
    });
    verifiedOnly = await seedFinding(admin, org, runId, correctness, {
      reviewer: 'correctness',
      severity: 'medium',
      verified: { confidence: 0.6, band: 'internal', stageOutcomes: STAGES },
    });
    lowConfidence = await seedFinding(admin, org, runId, correctness, {
      reviewer: 'correctness',
      severity: 'low',
      state: 'SUPPRESSED_LOW_CONFIDENCE',
      stage: 7,
      suppression: {
        reason: { type: 'low_confidence', computed: 0.4, threshold: 0.55 },
        detail: 'computed confidence below threshold',
        stage: 7,
      },
    });
    duplicate = await seedFinding(admin, org, runId, security, {
      severity: 'critical',
      state: 'SUPPRESSED_DUPLICATE',
      stage: 8,
      title: 'Authorization bypass (duplicate)',
      suppression: {
        reason: { type: 'duplicate', of: published.candidateId },
        detail: 'same fingerprint',
        stage: 8,
      },
    });
    app = await createApiApp();
  });

  afterAll(async () => {
    await app.close();
    await cleanup(admin, [org.organizationId, other.organizationId], [viewer]);
    await admin.destroy();
  });

  const ids = (body: { items: { id: string }[] }) => body.items.map((i) => i.id);

  it('findings_filter_by_state', async () => {
    const all = await as(app, viewer).get(`/reviews/${runId}/findings`).expect(200);
    expect(all.body.items).toHaveLength(4);
    // Severity first: the two critical findings lead.
    expect(all.body.items.slice(0, 2).map((i: { severity: string }) => i.severity)).toEqual([
      'critical',
      'critical',
    ]);
    const pub = await as(app, viewer).get(`/reviews/${runId}/findings?state=published`).expect(200);
    expect(ids(pub.body)).toEqual([published.verifiedId]);
    expect(pub.body.items[0]).toMatchObject({ lifecycle: 'published', confidence: 0.92 });
    const verified = await as(app, viewer)
      .get(`/reviews/${runId}/findings?state=verified`)
      .expect(200);
    expect(new Set(ids(verified.body))).toEqual(
      new Set([published.verifiedId, verifiedOnly.verifiedId]),
    );
    const suppressed = await as(app, viewer)
      .get(`/reviews/${runId}/findings?state=suppressed`)
      .expect(200);
    expect(new Set(ids(suppressed.body))).toEqual(
      new Set([lowConfidence.candidateId, duplicate.candidateId]),
    );
    const bySeverity = await as(app, viewer)
      .get(`/reviews/${runId}/findings?severity=medium,low&reviewer=correctness`)
      .expect(200);
    expect(new Set(ids(bySeverity.body))).toEqual(
      new Set([verifiedOnly.verifiedId, lowConfidence.candidateId]),
    );
    await as(app, viewer).get(`/reviews/${runId}/findings?state=bogus`).expect(400);
  });

  it('suppressed_findings_have_reason', async () => {
    const res = await as(app, viewer)
      .get(`/reviews/${runId}/findings?state=suppressed`)
      .expect(200);
    for (const item of res.body.items as { suppression: unknown }[]) {
      expect(item.suppression).toMatchObject({
        reason: expect.any(String),
        stage: expect.any(Number),
      });
    }
    const dup = (res.body.items as { id: string; suppression: { duplicate_of: string } }[]).find(
      (i) => i.id === duplicate.candidateId,
    );
    expect(dup?.suppression).toMatchObject({
      reason: 'duplicate',
      duplicate_of: published.candidateId,
    });
  });

  it('finding detail carries anchor, evidence, confidence and publication', async () => {
    const res = await as(app, viewer).get(`/findings/${published.verifiedId}`).expect(200);
    expect(res.body).toMatchObject({
      id: published.verifiedId,
      anchor: { path: 'src/users/user.controller.ts', start_line: 10, end_line: 14 },
      symbols: ['UserController.update'],
      severity: 'critical',
      confidence_detail: { value: 0.92 },
      publication: { placement: 'inline' },
    });
    expect(res.body.evidence[0]).toMatchObject({ kind: 'caller_path' });
    // A suppressed candidate is addressable by its candidate id.
    await as(app, viewer).get(`/findings/${lowConfidence.candidateId}`).expect(200);
  });

  it('trace_contains_all_stages', async () => {
    const res = await as(app, viewer).get(`/findings/${published.verifiedId}/trace`).expect(200);
    expect(res.body.verification.stages).toHaveLength(8);
    expect(res.body.symbol_path).toEqual([
      'UserController.update',
      'AdminService.updateUser',
      'AuthService.authorize',
    ]);
    expect(res.body.reviewer).toMatchObject({ type: 'security', version: 'security:v1' });
    expect(res.body.dedup).toEqual({ merged_into: null, merged_from: [duplicate.candidateId] });
    expect(res.body.policy.band).toBe('publish');
    expect(res.body.publication).toMatchObject({ placement: 'inline' });
    expect(res.body.context_refs).toEqual([
      { path: 'src/users/user.controller.ts', start_line: 10, end_line: 14, snapshot_id: null },
    ]);
    expect(res.body.incomplete).toBe(false);

    // A run that recorded fewer stages is returned anyway, marked incomplete.
    const pr = await seedPullRequest(admin, org);
    const oldRun = await seedReviewRun(admin, org, pr);
    const rr = await seedReviewerRun(admin, org, oldRun, 'security', 'succeeded');
    const old = await seedFinding(admin, org, oldRun, rr, {
      verified: { stageOutcomes: STAGES.slice(0, 3) },
    });
    const partial = await as(app, viewer).get(`/findings/${old.verifiedId}/trace`).expect(200);
    expect(partial.body.incomplete).toBe(true);
    expect(partial.body.verification.stages).toHaveLength(3);
  });

  it('trace_excludes_prompt_and_raw_output', async () => {
    for (const path of [
      `/findings/${published.verifiedId}/trace`,
      `/findings/${published.verifiedId}`,
      `/reviews/${runId}/findings`,
    ]) {
      const res = await as(app, viewer).get(path).expect(200);
      const text = JSON.stringify(res.body);
      expect(text).not.toContain('MODEL-RAW-OUTPUT');
      expect(text).not.toContain('SYSTEM-PROMPT');
      expect(text).not.toContain('SECRET-REASONING');
      expect(text).not.toMatch(/"(prompt|raw_output|reasoning_artifacts)"/);
    }
  });

  it('foreign_finding_404', async () => {
    const pr = await seedPullRequest(admin, other);
    const otherRun = await seedReviewRun(admin, other, pr);
    const rr = await seedReviewerRun(admin, other, otherRun, 'security', 'succeeded');
    const foreign = await seedFinding(admin, other, otherRun, rr, { verified: {} });
    await as(app, viewer).get(`/findings/${foreign.verifiedId}`).expect(404);
    await as(app, viewer).get(`/findings/${foreign.candidateId}/trace`).expect(404);
    await as(app, viewer).get(`/reviews/${otherRun}/findings`).expect(404);
    await as(app, viewer).get('/findings/00000000-0000-4000-8000-000000000000').expect(404);
  });
});
