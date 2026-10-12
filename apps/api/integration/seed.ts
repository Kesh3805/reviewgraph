import { randomInt, randomUUID } from 'node:crypto';
import type { Kysely } from 'kysely';
import type { DB } from '../src/db/generated';
import { createKysely, createPool } from '../src/db/kysely.provider';

/** Plain (not role-switched) connection used to seed and clean up; the dev login is a superuser. */
export function adminDb(max = 2): Kysely<DB> {
  return createKysely(createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max }));
}

export interface SeededOrg {
  organizationId: string;
  installationId: string;
  providerInstallationId: number;
  repositoryIds: string[];
  slug: string;
}

export const unique = (prefix: string): string => `${prefix}-${randomUUID().slice(0, 8)}`;

export function newProviderInstallationId(): number {
  return randomInt(1_000_000_000, 2_000_000_000);
}

/** One organization with one installation and `repos` repositories. */
export async function seedOrg(db: Kysely<DB>, repos = 1): Promise<SeededOrg> {
  const slug = unique('it');
  const org = await db
    .insertInto('organizations')
    .values({ slug, display_name: slug })
    .returning('id')
    .executeTakeFirstOrThrow();
  const providerInstallationId = newProviderInstallationId();
  const installation = await db
    .insertInto('provider_installations')
    .values({
      organization_id: org.id,
      provider: 'github',
      provider_installation_id: providerInstallationId,
      account_login: slug,
      account_type: 'organization',
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  const repositoryIds: string[] = [];
  for (let i = 0; i < repos; i++) {
    const repo = await db
      .insertInto('repositories')
      .values({
        organization_id: org.id,
        installation_id: installation.id,
        provider: 'github',
        provider_repo_id: String(randomInt(1, 2_000_000_000)),
        full_name: `${slug}/repo-${i}`,
        default_branch: 'main',
        visibility: 'private',
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    repositoryIds.push(repo.id);
  }
  return {
    organizationId: org.id,
    installationId: installation.id,
    providerInstallationId,
    repositoryIds,
    slug,
  };
}

export async function seedUser(db: Kysely<DB>): Promise<string> {
  const login = unique('user');
  const user = await db
    .insertInto('users')
    .values({ provider: 'github', provider_user_id: String(randomInt(1, 2_000_000_000)), login })
    .returning('id')
    .executeTakeFirstOrThrow();
  return user.id;
}

export async function addMember(
  db: Kysely<DB>,
  organizationId: string,
  userId: string,
  role: 'owner' | 'admin' | 'member' | 'viewer',
): Promise<void> {
  await db
    .insertInto('memberships')
    .values({ organization_id: organizationId, user_id: userId, role })
    .execute();
}

export const sha = (c: string): string => c.repeat(40).slice(0, 40);

let prNumber = 1;

/** A pull request of a seeded repository (open by default). */
export async function seedPullRequest(
  db: Kysely<DB>,
  org: SeededOrg,
  overrides: { repoIndex?: number; state?: string; headSha?: string } = {},
): Promise<string> {
  const pr = await db
    .insertInto('pull_requests')
    .values({
      organization_id: org.organizationId,
      repository_id: org.repositoryIds[overrides.repoIndex ?? 0]!,
      provider_number: prNumber++,
      title: 'Add the thing',
      author_login: 'octocat',
      base_ref: 'main',
      head_ref: 'feature',
      base_sha: sha('b'),
      head_sha: overrides.headSha ?? sha('a'),
      state: overrides.state ?? 'open',
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  return pr.id;
}

export async function seedReviewRun(
  db: Kysely<DB>,
  org: SeededOrg,
  pullRequestId: string,
  overrides: Partial<{
    state: string;
    headSha: string;
    degradedReviewers: string[];
    createdAt: Date;
    traceParent: string;
    failureClass: string;
  }> = {},
): Promise<string> {
  const pr = await db
    .selectFrom('pull_requests')
    .select(['repository_id', 'head_sha'])
    .where('id', '=', pullRequestId)
    .executeTakeFirstOrThrow();
  const run = await db
    .insertInto('review_runs')
    .values({
      organization_id: org.organizationId,
      repository_id: pr.repository_id,
      pull_request_id: pullRequestId,
      base_sha: sha('b'),
      head_sha: overrides.headSha ?? pr.head_sha,
      state: overrides.state ?? 'COMPLETED',
      trigger: 'webhook',
      degraded_reviewers: overrides.degradedReviewers ?? [],
      trace_parent: overrides.traceParent ?? null,
      failure_class: overrides.failureClass ?? null,
      ...(overrides.createdAt ? { created_at: overrides.createdAt } : {}),
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  return run.id;
}

export async function seedReviewerRun(
  db: Kysely<DB>,
  org: SeededOrg,
  reviewRunId: string,
  reviewer: string,
  state: string,
  overrides: { clusterKey?: string; errorClass?: string } = {},
): Promise<string> {
  const failed = state === 'failed' || state === 'timed_out';
  const row = await db
    .insertInto('reviewer_runs')
    .values({
      organization_id: org.organizationId,
      review_run_id: reviewRunId,
      reviewer,
      state,
      cluster_key: overrides.clusterKey ?? null,
      error_class: overrides.errorClass ?? (failed ? 'transient' : null),
      provider: 'replay',
      model: 'replay-model',
      prompt_version: `${reviewer}:v1`,
      reviewer_version: `${reviewer}:v1`,
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  return row.id;
}

/** A job for a review run, as the pipeline would have enqueued it. */
export async function seedReviewJob(
  db: Kysely<DB>,
  org: SeededOrg,
  reviewRunId: string,
  state = 'queued',
): Promise<string> {
  const job = await db
    .insertInto('jobs')
    .values({
      organization_id: org.organizationId,
      queue: 'pr-review',
      idempotency_key: unique(`pr-review:seed:${reviewRunId}`),
      payload: JSON.stringify({ review_run_id: reviewRunId }),
      state,
      locked_by: state === 'running' ? 'seed-worker' : null,
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  return job.id;
}

export interface SeededFinding {
  candidateId: string;
  verifiedId: string | null;
  publishedId: string | null;
}

/**
 * A finding of a review run: a candidate, plus its verified row and its publication when asked.
 * Suppressed states need a `suppression`.
 */
export async function seedFinding(
  db: Kysely<DB>,
  org: SeededOrg,
  reviewRunId: string,
  reviewerRunId: string,
  opts: {
    reviewer?: string;
    state?: string;
    severity?: string;
    title?: string;
    evidence?: unknown[];
    suppression?: unknown;
    stage?: number;
    verified?: { severity?: string; confidence?: number; stageOutcomes?: unknown; band?: string };
    published?: boolean;
  } = {},
): Promise<SeededFinding> {
  const evidence = JSON.stringify(opts.evidence ?? []);
  const candidate = await db
    .insertInto('candidate_findings')
    .values({
      organization_id: org.organizationId,
      review_run_id: reviewRunId,
      reviewer_run_id: reviewerRunId,
      reviewer: opts.reviewer ?? 'security',
      category: 'security',
      title: opts.title ?? 'Authorization bypass',
      description: 'The update path skips the authorization check.',
      changed_path: 'src/users/user.controller.ts',
      changed_side: 'head',
      changed_start_line: 10,
      changed_end_line: 14,
      severity_candidate: opts.severity ?? 'high',
      affected_symbols: ['UserController.update'],
      evidence,
      reasoning_artifacts: JSON.stringify([{ kind: 'raw_output', summary: 'SECRET-REASONING' }]),
      fingerprint: `v1:${randomUUID().replace(/-/g, '')}`,
      state: opts.state ?? (opts.published ? 'PUBLISHED' : 'VERIFIED'),
      suppression: opts.suppression === undefined ? null : JSON.stringify(opts.suppression),
      suppressed_at_stage: opts.stage ?? null,
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  let verifiedId: string | null = null;
  let publishedId: string | null = null;
  if (opts.verified || opts.published) {
    const v = opts.verified ?? {};
    const verified = await db
      .insertInto('verified_findings')
      .values({
        organization_id: org.organizationId,
        candidate_finding_id: candidate.id,
        review_run_id: reviewRunId,
        computed_confidence: v.confidence ?? 0.9,
        severity: v.severity ?? opts.severity ?? 'high',
        band: v.band ?? 'publish',
        verification_version: 1,
        stage_outcomes: JSON.stringify(v.stageOutcomes ?? []),
        evidence,
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    verifiedId = verified.id;
  }
  if (opts.published && verifiedId) {
    const run = await db
      .selectFrom('review_runs')
      .select(['pull_request_id', 'head_sha'])
      .where('id', '=', reviewRunId)
      .executeTakeFirstOrThrow();
    const published = await db
      .insertInto('published_findings')
      .values({
        organization_id: org.organizationId,
        verified_finding_id: verifiedId,
        review_run_id: reviewRunId,
        pull_request_id: run.pull_request_id,
        provider: 'github',
        placement: 'inline',
        path: 'src/users/user.controller.ts',
        start_line: 10,
        end_line: 14,
        head_sha: run.head_sha,
        provider_review_id: String(randomInt(1, 2_000_000_000)),
        provider_comment_id: String(randomInt(1, 2_000_000_000)),
        published_at: new Date(),
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    publishedId = published.id;
  }
  return { candidateId: candidate.id, verifiedId, publishedId };
}

/** Deleting the organization cascades to every tenant row. */
export async function cleanup(
  db: Kysely<DB>,
  orgIds: string[],
  userIds: string[] = [],
): Promise<void> {
  if (orgIds.length) await db.deleteFrom('organizations').where('id', 'in', orgIds).execute();
  if (userIds.length) await db.deleteFrom('users').where('id', 'in', userIds).execute();
}
