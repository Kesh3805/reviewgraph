import {
  ConflictException,
  HttpStatus,
  Inject,
  Injectable,
  NotFoundException,
} from '@nestjs/common';
import { sql, type Selectable } from 'kysely';
import { AuditService } from '../audit/audit.service';
import { decodeTimeCursor, pageOf } from '../common/cursor';
import { incCounter } from '../common/metrics';
import { ProblemException } from '../common/problem.filter';
import { DbService } from '../db/db.module';
import type { DB } from '../db/generated';
import type { Tx } from '../db/tx';
import { JOB_QUEUE, type JobQueue } from '../jobs/job-queue';
import {
  ACTIVE_REVIEW_STATES,
  type CancelReviewResult,
  type Completeness,
  type PullRequestResponse,
  type ReviewDetailResponse,
  type ReviewState,
  type ReviewSummaryResponse,
  type StartReviewResult,
} from './dto/review.dto';
import { SupersessionService } from './supersession.service';

/** Postgres microsecond timestamp text used by the keyset cursor. */
const cursorTs = (column: string) =>
  sql<string>`to_char(${sql.ref(column)} at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')`;

type RunRow = Selectable<DB['review_runs']>;
type ReviewerRunRow = Selectable<DB['reviewer_runs']>;

export interface Actor {
  userId: string;
}

/** Pull requests and review runs (API-009). Every method runs tenant scoped. */
@Injectable()
export class ReviewsService {
  constructor(
    private readonly dbs: DbService,
    private readonly audit: AuditService,
    private readonly supersession: SupersessionService,
    @Inject(JOB_QUEUE) private readonly queue: JobQueue,
  ) {}

  async listPullRequests(
    orgId: string,
    repositoryId: string,
    query: { state: 'open' | 'closed' | 'all'; limit: number; cursor?: string },
  ): Promise<{ items: PullRequestResponse[]; next_cursor: string | null }> {
    const after = decodeTimeCursor(query.cursor);
    const rows = await this.dbs.withTx(orgId, async (trx) => {
      const repo = await trx
        .selectFrom('repositories')
        .select('id')
        .where('id', '=', repositoryId)
        .executeTakeFirst();
      if (!repo) throw new NotFoundException();
      let q = trx
        .selectFrom('pull_requests as p')
        .selectAll('p')
        .select(cursorTs('p.created_at').as('cursor_ts'))
        .select((eb) =>
          eb
            .selectFrom('review_runs as rr')
            .select(
              sql<unknown>`json_build_object('id', rr.id, 'state', rr.state, 'head_sha', rr.head_sha,
                'created_at', ${cursorTs('rr.created_at')})`.as('j'),
            )
            .whereRef('rr.pull_request_id', '=', 'p.id')
            .orderBy('rr.created_at', 'desc')
            .orderBy('rr.id', 'desc')
            .limit(1)
            .as('latest_review'),
        )
        .where('p.repository_id', '=', repositoryId);
      if (query.state === 'open') q = q.where('p.state', '=', 'open');
      if (query.state === 'closed') q = q.where('p.state', 'in', ['closed', 'merged']);
      if (after) {
        q = q.where(
          sql<boolean>`(p.created_at, p.id) < (${after.createdAt}::timestamptz, ${after.id}::uuid)`,
        );
      }
      return q
        .orderBy('p.created_at', 'desc')
        .orderBy('p.id', 'desc')
        .limit(query.limit + 1)
        .execute();
    });
    const { page, next_cursor } = pageOf(rows, query.limit);
    return { items: page.map((r) => toPullRequest(r, r.latest_review)), next_cursor };
  }

  async getPullRequest(orgId: string, pullRequestId: string): Promise<PullRequestResponse> {
    return this.dbs.withTx(orgId, async (trx) => {
      const pr = await trx
        .selectFrom('pull_requests')
        .selectAll()
        .where('id', '=', pullRequestId)
        .executeTakeFirst();
      if (!pr) throw new NotFoundException();
      const latest = await trx
        .selectFrom('review_runs')
        .select(['id', 'state', 'head_sha', cursorTs('created_at').as('created_at')])
        .where('pull_request_id', '=', pullRequestId)
        .orderBy('created_at', 'desc')
        .orderBy('id', 'desc')
        .executeTakeFirst();
      return toPullRequest(pr, latest ?? null);
    });
  }

  async listReviews(
    orgId: string,
    pullRequestId: string,
    query: { limit: number; cursor?: string },
  ): Promise<{ items: ReviewSummaryResponse[]; next_cursor: string | null }> {
    const after = decodeTimeCursor(query.cursor);
    const rows = await this.dbs.withTx(orgId, async (trx) => {
      await this.loadPullRequest(trx, pullRequestId);
      let q = trx
        .selectFrom('review_runs')
        .selectAll()
        .select(cursorTs('created_at').as('cursor_ts'))
        .where('pull_request_id', '=', pullRequestId);
      if (after) {
        q = q.where(
          sql<boolean>`(created_at, id) < (${after.createdAt}::timestamptz, ${after.id}::uuid)`,
        );
      }
      return q
        .orderBy('created_at', 'desc')
        .orderBy('id', 'desc')
        .limit(query.limit + 1)
        .execute();
    });
    const { page, next_cursor } = pageOf(rows, query.limit);
    return { items: page.map(toReviewSummary), next_cursor };
  }

  async getReview(
    orgId: string,
    reviewId: string,
    pullRequestId?: string,
  ): Promise<ReviewDetailResponse> {
    return this.dbs.withTx(orgId, async (trx) => {
      const run = await trx
        .selectFrom('review_runs')
        .selectAll()
        .where('id', '=', reviewId)
        .executeTakeFirst();
      if (!run || (pullRequestId && run.pull_request_id !== pullRequestId)) {
        throw new NotFoundException();
      }
      const reviewerRuns = await trx
        .selectFrom('reviewer_runs')
        .selectAll()
        .where('review_run_id', '=', reviewId)
        .orderBy('reviewer')
        .orderBy('cluster_key')
        .execute();
      const counts = await trx
        .selectFrom('candidate_findings')
        .select(['state', (eb) => eb.fn.countAll<string>().as('n')])
        .where('review_run_id', '=', reviewId)
        .groupBy('state')
        .execute();
      const published = await trx
        .selectFrom('published_findings')
        .select((eb) => eb.fn.countAll<string>().as('n'))
        .where('review_run_id', '=', reviewId)
        .executeTakeFirstOrThrow();
      return toReviewDetail(run, reviewerRuns, counts, Number(published.n));
    });
  }

  /** Manual trigger at the current head: the same path as webhooks (SUP-001). */
  async startManualReview(
    orgId: string,
    actor: Actor,
    pullRequestId: string,
  ): Promise<StartReviewResult> {
    const outcome = await this.dbs.withTx(orgId, async (trx) => {
      const pr = await trx
        .selectFrom('pull_requests')
        .select(['head_sha', 'base_sha', 'state'])
        .where('id', '=', pullRequestId)
        .executeTakeFirst();
      if (!pr) throw new NotFoundException();
      if (pr.state !== 'open') throw new ConflictException(`the pull request is ${pr.state}`);
      const result = await this.supersession.startInTx(trx, {
        organizationId: orgId,
        pullRequestId,
        headSha: pr.head_sha,
        baseSha: pr.base_sha,
        trigger: 'manual',
        // One manual review per head and minute.
        triggerSuffix: `manual:${minuteBucket(new Date())}`,
      });
      if (result.outcome === 'not_found') throw new NotFoundException();
      if (!('reviewRunId' in result)) {
        throw new ConflictException('the pull request cannot be reviewed now');
      }
      const job = await trx
        .selectFrom('jobs')
        .select('id')
        .where(sql<boolean>`payload ->> 'review_run_id' = ${result.reviewRunId}`)
        .where('queue', '=', 'pr-review')
        .orderBy('created_at', 'desc')
        .executeTakeFirst();
      const created = result.outcome === 'started';
      if (created) {
        await this.audit.record(trx, {
          organizationId: orgId,
          actor: { type: 'user', id: actor.userId },
          action: 'review.manual_triggered',
          targetType: 'pull_request',
          targetId: pullRequestId,
          metadata: {
            review_run_id: result.reviewRunId,
            head_sha: pr.head_sha,
            superseded_run_ids: result.superseded,
          },
        });
      }
      return {
        review_run_id: result.reviewRunId,
        job_id: job?.id ?? null,
        created,
        head_sha: pr.head_sha,
        superseded_run_ids: result.superseded,
      };
    });
    if (outcome.created) incCounter('manual_reviews_total');
    return outcome;
  }

  /**
   * Cancels a run by compare-and-set and cancels its queued jobs. Cancelling a cancelled run is
   * a no-op; any other terminal state answers 409 with the current state.
   */
  async cancel(orgId: string, actor: Actor, reviewId: string): Promise<CancelReviewResult> {
    return this.dbs.withTx(orgId, async (trx) => {
      const updated = await trx
        .updateTable('review_runs')
        .set({ state: 'CANCELLED', completed_at: sql<Date>`now()` })
        .where('id', '=', reviewId)
        .where('state', 'in', [...ACTIVE_REVIEW_STATES])
        .returning(['id', 'repository_id'])
        .executeTakeFirst();
      if (!updated) {
        const current = await trx
          .selectFrom('review_runs')
          .select('state')
          .where('id', '=', reviewId)
          .executeTakeFirst();
        if (!current) throw new NotFoundException();
        if (current.state === 'CANCELLED') {
          return { review_run_id: reviewId, state: 'CANCELLED', cancelled_jobs: 0 };
        }
        throw new ProblemException(
          HttpStatus.CONFLICT,
          `the review run is already ${current.state}`,
          { state: current.state },
        );
      }
      const cancelledJobs = await this.queue.cancelWhere(trx, { reviewRunIds: [reviewId] });
      await this.audit.record(trx, {
        organizationId: orgId,
        repositoryId: updated.repository_id,
        actor: { type: 'user', id: actor.userId },
        action: 'review.cancelled',
        targetType: 'review_run',
        targetId: reviewId,
        metadata: { cancelled_jobs: cancelledJobs },
      });
      return { review_run_id: reviewId, state: 'CANCELLED', cancelled_jobs: cancelledJobs };
    });
  }

  private async loadPullRequest(trx: Tx, pullRequestId: string): Promise<void> {
    const pr = await trx
      .selectFrom('pull_requests')
      .select('id')
      .where('id', '=', pullRequestId)
      .executeTakeFirst();
    if (!pr) throw new NotFoundException();
  }
}

interface LatestReview {
  id: string;
  state: string;
  head_sha: string;
  created_at: string;
}

function toPullRequest(pr: Selectable<DB['pull_requests']>, latest: unknown): PullRequestResponse {
  const review = latest as LatestReview | null;
  return {
    id: pr.id,
    repository_id: pr.repository_id,
    number: pr.provider_number,
    title: pr.title,
    author_login: pr.author_login,
    base_ref: pr.base_ref,
    head_ref: pr.head_ref,
    base_sha: pr.base_sha,
    head_sha: pr.head_sha,
    state: pr.state as PullRequestResponse['state'],
    draft: pr.draft,
    created_at: pr.created_at.toISOString(),
    updated_at: pr.updated_at.toISOString(),
    latest_review: review
      ? {
          id: review.id,
          state: review.state as ReviewState,
          head_sha: review.head_sha,
          created_at: review.created_at,
        }
      : null,
  };
}

function toReviewSummary(run: RunRow): ReviewSummaryResponse {
  return {
    id: run.id,
    pull_request_id: run.pull_request_id,
    repository_id: run.repository_id,
    state: run.state as ReviewState,
    trigger: run.trigger as ReviewSummaryResponse['trigger'],
    base_sha: run.base_sha,
    head_sha: run.head_sha,
    superseded_by: run.superseded_by,
    retry_of: run.retry_of,
    degraded_reviewers: run.degraded_reviewers,
    created_at: run.created_at.toISOString(),
    updated_at: run.updated_at.toISOString(),
    completed_at: run.completed_at ? run.completed_at.toISOString() : null,
  };
}

/**
 * Completeness from the reviewer runs (INV-013): a reviewer succeeded when every one of its
 * runs (one per cluster) succeeded, failed when any run failed or timed out, and was not
 * executed when its runs were skipped or never started.
 */
export function computeCompleteness(
  rows: Pick<ReviewerRunRow, 'reviewer' | 'state' | 'error_class'>[],
): Completeness {
  const byReviewer = new Map<string, Pick<ReviewerRunRow, 'state' | 'error_class'>[]>();
  for (const row of rows) {
    const list = byReviewer.get(row.reviewer) ?? [];
    list.push(row);
    byReviewer.set(row.reviewer, list);
  }
  let succeeded = 0;
  const failed: Completeness['reviewers_failed'] = [];
  const notExecuted: string[] = [];
  for (const [reviewer, list] of [...byReviewer].sort(([a], [b]) => a.localeCompare(b))) {
    const failure = list.find((r) => r.state === 'failed' || r.state === 'timed_out');
    if (failure) failed.push({ reviewer, error_class: failure.error_class });
    else if (list.every((r) => r.state === 'succeeded')) succeeded++;
    else if (list.some((r) => r.state === 'skipped' || r.state === 'pending')) {
      notExecuted.push(reviewer);
    }
  }
  return {
    reviewers_planned: byReviewer.size,
    reviewers_succeeded: succeeded,
    reviewers_failed: failed,
    not_executed: notExecuted,
  };
}

/** The W3C trace id of a `traceparent`. */
export function traceIdOf(traceParent: string | null): string | null {
  const match = traceParent ? /^[0-9a-f]{2}-([0-9a-f]{32})-/.exec(traceParent) : null;
  return match ? match[1]! : null;
}

function provenanceField(provenance: unknown, key: string): unknown {
  if (!provenance || typeof provenance !== 'object' || Array.isArray(provenance)) return null;
  return (provenance as Record<string, unknown>)[key] ?? null;
}

function toCoverage(value: unknown): ReviewDetailResponse['coverage'] {
  if (!value || typeof value !== 'object') return null;
  const { reviewed_clusters, unreviewed_clusters } = value as Record<string, unknown>;
  const strings = (v: unknown): string[] =>
    Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  return {
    reviewed_clusters: strings(reviewed_clusters),
    unreviewed_clusters: strings(unreviewed_clusters),
  };
}

function toStages(value: unknown): ReviewDetailResponse['stages'] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((s: unknown) => {
    if (!s || typeof s !== 'object') return [];
    const { state, entered_at, duration_ms } = s as Record<string, unknown>;
    if (typeof state !== 'string' || typeof entered_at !== 'string') return [];
    return [
      { state, entered_at, duration_ms: typeof duration_ms === 'number' ? duration_ms : null },
    ];
  });
}

function toReviewDetail(
  run: RunRow,
  reviewerRuns: ReviewerRunRow[],
  counts: { state: string; n: string }[],
  published: number,
): ReviewDetailResponse {
  const completeness = computeCompleteness(reviewerRuns);
  const degradedReviewers = [
    ...new Set([
      ...run.degraded_reviewers,
      ...completeness.reviewers_failed.map((f) => f.reviewer),
    ]),
  ].sort();
  return {
    ...toReviewSummary(run),
    failure: run.failure_class ? { class: run.failure_class, detail: run.failure_detail } : null,
    stages: toStages(provenanceField(run.provenance, 'stages')),
    reviewer_runs: reviewerRuns.map((r) => ({
      id: r.id,
      reviewer: r.reviewer,
      cluster_key: r.cluster_key,
      state: r.state as ReviewDetailResponse['reviewer_runs'][number]['state'],
      error_class: r.error_class,
      provider: r.provider,
      model: r.model,
      prompt_version: r.prompt_version,
      reviewer_version: r.reviewer_version,
      input_tokens: Number(r.input_tokens),
      output_tokens: Number(r.output_tokens),
      cost_usd_micros: Number(r.cost_usd_micros),
      latency_ms: r.latency_ms,
      started_at: r.started_at ? r.started_at.toISOString() : null,
      finished_at: r.finished_at ? r.finished_at.toISOString() : null,
    })),
    completeness,
    degraded: {
      degraded: degradedReviewers.length > 0,
      reviewers: degradedReviewers,
      reasons: completeness.reviewers_failed,
    },
    risk_assessment: provenanceField(run.provenance, 'risk_assessment'),
    change_summary: provenanceField(run.provenance, 'change_summary'),
    coverage: toCoverage(provenanceField(run.provenance, 'coverage')),
    finding_counts: {
      by_state: Object.fromEntries(counts.map((c) => [c.state, Number(c.n)])),
      published,
    },
    trace_id: traceIdOf(run.trace_parent),
  };
}

function minuteBucket(date: Date): string {
  return date.toISOString().slice(0, 16).replace(/[-T:]/g, '');
}
