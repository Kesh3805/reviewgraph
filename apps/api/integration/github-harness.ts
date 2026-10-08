import { randomInt, randomUUID } from 'node:crypto';
import type { NestExpressApplication } from '@nestjs/platform-express';
import type { TestingModuleBuilder } from '@nestjs/testing';
import { Redis } from 'ioredis';
import type { Kysely } from 'kysely';
import { sql } from 'kysely';
import type { DB } from '../src/db/generated';
import type { Tx } from '../src/db/tx';
import { GithubPermissionsMonitor } from '../src/providers/github/permissions-monitor';
import { REVIEW_JOBS, type PrReviewJob, type ReviewJobs } from '../src/reviews/review-jobs.port';
import { createTestApp } from '../test/helpers';
import { generateAppKey, type TestAppKey } from '../test/helpers/fake-github';
import { FakeGithubApi } from '../test/helpers/fake-github-api';
import { adminDb, seedOrg, type SeededOrg } from './seed';

export const WEBHOOK_SECRET = 'whsec_github_harness_0123456789';
export const APP_SLUG = 'reviewgraph';

export const sha = (c: string): string => c.repeat(40);

/** Records enqueues and cancellations instead of writing the (not yet present) jobs table. */
export class RecordingReviewJobs implements ReviewJobs {
  readonly enqueued: PrReviewJob[] = [];
  readonly cancelled: string[] = [];
  failEnqueue = false;

  enqueuePrReview(_trx: Tx, job: PrReviewJob): Promise<{ created: boolean }> {
    if (this.failEnqueue) return Promise.reject(new Error('queue down'));
    this.enqueued.push(job);
    return Promise.resolve({ created: true });
  }

  cancelQueuedForRuns(_trx: Tx, ids: string[]): Promise<number> {
    this.cancelled.push(...ids);
    return Promise.resolve(ids.length);
  }

  reset(): void {
    this.enqueued.length = 0;
    this.cancelled.length = 0;
    this.failEnqueue = false;
  }
}

export interface Harness {
  app: NestExpressApplication;
  github: FakeGithubApi;
  key: TestAppKey;
  admin: Kysely<DB>;
  redis: Redis;
  jobs: RecordingReviewJobs;
  orgs: string[];
  close(): Promise<void>;
}

/** App with GitHub enabled against the stateful fake GitHub API and the real Postgres/Redis. */
export async function startHarness(
  opts: {
    env?: NodeJS.ProcessEnv;
    configure?: (b: TestingModuleBuilder) => TestingModuleBuilder;
  } = {},
): Promise<Harness> {
  const key = generateAppKey();
  const github = new FakeGithubApi(key.publicKey);
  await github.start();
  const jobs = new RecordingReviewJobs();
  const appRedis = new Redis(process.env.RG_TEST_REDIS_URL!);
  const app = await createTestApp(undefined, [], {
    redis: appRedis,
    env: {
      DATABASE_URL: process.env.RG_TEST_DATABASE_URL!,
      DB_APP_ROLE: 'rg_api',
      GITHUB_ENABLED: 'true',
      GITHUB_APP_ID: '1',
      GITHUB_APP_SLUG: APP_SLUG,
      GITHUB_APP_PRIVATE_KEY: key.pem.replace(/\n/g, '\\n'),
      GITHUB_WEBHOOK_SECRET: WEBHOOK_SECRET,
      GITHUB_CLIENT_ID: 'c',
      GITHUB_CLIENT_SECRET: 's',
      GITHUB_API_URL: github.url,
      RECONCILER_ENABLED: 'false',
      ...opts.env,
    },
    configure: (builder) => {
      let b = builder
        .overrideProvider(GithubPermissionsMonitor)
        .useValue({ enabled: false, status: () => 'unknown', verify: () => Promise.resolve() })
        .overrideProvider(REVIEW_JOBS)
        .useValue(jobs);
      if (opts.configure) b = opts.configure(b);
      return b;
    },
  });
  await app.init();
  const admin = adminDb(4);
  const redis = new Redis(process.env.RG_TEST_REDIS_URL!);
  const orgs: string[] = [];
  return {
    app,
    github,
    key,
    admin,
    redis,
    jobs,
    orgs,
    async close() {
      await app.close();
      await github.stop();
      if (orgs.length) await admin.deleteFrom('organizations').where('id', 'in', orgs).execute();
      await admin.destroy();
      redis.disconnect();
    },
  };
}

export interface SeededPr {
  org: SeededOrg;
  repositoryId: string;
  fullName: string;
  owner: string;
  name: string;
  providerRepoId: number;
  installationId: string;
  pullRequestId: string;
  number: number;
}

/** An organization, installation and repository known to both Postgres and the fake. */
export async function seedRepo(h: Harness): Promise<Omit<SeededPr, 'pullRequestId' | 'number'>> {
  const org = await seedOrg(h.admin, 1);
  h.orgs.push(org.organizationId);
  const repo = await h.admin
    .selectFrom('repositories')
    .select(['id', 'full_name', 'provider_repo_id'])
    .where('id', '=', org.repositoryIds[0]!)
    .executeTakeFirstOrThrow();
  const [owner, name] = repo.full_name.split('/') as [string, string];
  h.github.addRepo(repo.full_name, Number(repo.provider_repo_id));
  return {
    org,
    repositoryId: repo.id,
    fullName: repo.full_name,
    owner,
    name,
    providerRepoId: Number(repo.provider_repo_id),
    installationId: String(org.providerInstallationId),
  };
}

/** Repository plus an open pull request row at `head`. */
export async function seedPr(
  h: Harness,
  head = sha('a'),
  number = randomInt(1, 100_000),
): Promise<SeededPr> {
  const repo = await seedRepo(h);
  const pr = await h.admin
    .insertInto('pull_requests')
    .values({
      organization_id: repo.org.organizationId,
      repository_id: repo.repositoryId,
      provider_number: number,
      title: 't',
      author_login: 'octo-dev',
      base_ref: 'main',
      head_ref: 'feature',
      base_sha: sha('0'),
      head_sha: head,
      state: 'open',
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  h.github.setPull(repo.fullName, { number, headSha: head, baseSha: sha('0') });
  return { ...repo, pullRequestId: pr.id, number };
}

/** A run in `state` for the PR's head (as the engine would leave it). */
export async function seedRun(
  h: Harness,
  pr: SeededPr,
  state: string,
  head: string,
): Promise<string> {
  const row = await h.admin
    .insertInto('review_runs')
    .values({
      organization_id: pr.org.organizationId,
      repository_id: pr.repositoryId,
      pull_request_id: pr.pullRequestId,
      base_sha: sha('0'),
      head_sha: head,
      state,
      trigger: 'webhook',
      idempotency_key: `seed:${randomUUID()}`,
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  return row.id;
}

export interface SeedFinding {
  path: string;
  line: number;
  severity?: string;
  fingerprint?: string;
  title?: string;
  category?: string;
  symbols?: string[];
  explanation?: boolean;
}

export const fingerprint = (): string => `v1:${randomUUID().replace(/-/g, '')}`;

/** Prioritized, verified findings for a run, with complete PRD §59 explanations by default. */
export async function seedFindings(
  h: Harness,
  pr: SeededPr,
  runId: string,
  findings: SeedFinding[],
): Promise<{ verifiedIds: string[]; fingerprints: string[] }> {
  const reviewer = await h.admin
    .insertInto('reviewer_runs')
    .values({
      organization_id: pr.org.organizationId,
      review_run_id: runId,
      reviewer: 'correctness',
      state: 'succeeded',
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  const verifiedIds: string[] = [];
  const fingerprints: string[] = [];
  for (const [i, f] of findings.entries()) {
    const fp = f.fingerprint ?? fingerprint();
    const cand = await h.admin
      .insertInto('candidate_findings')
      .values({
        organization_id: pr.org.organizationId,
        review_run_id: runId,
        reviewer_run_id: reviewer.id,
        reviewer: 'correctness',
        category: f.category ?? 'correctness',
        title: f.title ?? `Finding ${i + 1}`,
        description: 'The total is charged twice.',
        changed_path: f.path,
        changed_side: 'head',
        changed_start_line: f.line,
        changed_end_line: f.line,
        severity_candidate: f.severity ?? 'high',
        affected_symbols: f.symbols ?? [],
        fingerprint: fp,
        state: 'PRIORITIZED',
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    const evidence =
      f.explanation === false
        ? []
        : {
            explanation: {
              what_changed: 'Invoice.total now calls charge().',
              why_risky: 'charge() has a side effect.',
              behavior_result: 'Customers are billed twice.',
              corrective_direction: 'Compute the sum without charging inside total().',
              evidence_path: ['Invoice.total', 'Invoice.charge'],
            },
          };
    const vf = await h.admin
      .insertInto('verified_findings')
      .values({
        organization_id: pr.org.organizationId,
        candidate_finding_id: cand.id,
        review_run_id: runId,
        computed_confidence: 0.9,
        severity: f.severity ?? 'high',
        band: 'publish',
        verification_version: 1,
        stage_outcomes: JSON.stringify({}),
        evidence: JSON.stringify(evidence),
        priority_score: 100 - i,
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    verifiedIds.push(vf.id);
    fingerprints.push(fp);
  }
  return { verifiedIds, fingerprints };
}

/** A unified-diff patch adding `count` lines from line 1 (every line is commentable). */
export function addedPatch(count: number): string {
  const lines = Array.from({ length: count }, (_, i) => `+line ${i + 1}`);
  return `@@ -0,0 +1,${count} @@\n${lines.join('\n')}`;
}

export async function runState(h: Harness, runId: string): Promise<string> {
  const row = await h.admin
    .selectFrom('review_runs')
    .select('state')
    .where('id', '=', runId)
    .executeTakeFirstOrThrow();
  return row.state;
}

/** Holds the PR row lock in a separate admin transaction until `release` is called. */
export async function holdPrLock(
  h: Harness,
  pullRequestId: string,
): Promise<{ release: () => Promise<void> }> {
  let release!: () => void;
  const released = new Promise<void>((r) => (release = r));
  let locked!: () => void;
  const isLocked = new Promise<void>((r) => (locked = r));
  const done = h.admin.transaction().execute(async (trx) => {
    await sql`select id from pull_requests where id = ${pullRequestId} for update`.execute(trx);
    locked();
    await released;
  });
  await isLocked;
  return {
    release: async () => {
      release();
      await done;
    },
  };
}
