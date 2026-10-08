import { Inject, Injectable, Logger } from '@nestjs/common';
import { SpanStatusCode, trace } from '@opentelemetry/api';
import { sql } from 'kysely';
import { v7 as uuidv7 } from 'uuid';
import { incCounter } from '../common/metrics';
import { DbService } from '../db/db.module';
import type { Tx } from '../db/tx';
import { TRACER_NAME } from '../telemetry/tracer.service';
import { REVIEW_JOBS, type ReviewJobs } from './review-jobs.port';

/** Run states that still hold (or wait for) a worker. Mirrors the one-active-run index. */
export const ACTIVE_RUN_STATES = [
  'RECEIVED',
  'INDEXING',
  'ANALYZING',
  'REVIEWING',
  'VERIFYING',
  'PUBLISHING',
] as const;

export type ReviewTrigger = 'webhook' | 'manual' | 'reconciler' | 'cli';
export type ReviewDepth = 'standard' | 'full';

/**
 * How long a head update waits for the pull request row. A publish in flight holds it for at
 * most ~15 s (SUP-003); the webhook path gives up after 2 s and is retried instead.
 */
export const SUPERSESSION_LOCK_TIMEOUT = '2s';

export interface StartReviewInput {
  organizationId: string;
  pullRequestId: string;
  headSha: string;
  baseSha: string;
  trigger: ReviewTrigger;
  depth?: ReviewDepth;
  /** The provider's `updated_at` of the event; an older event never moves the head. */
  prUpdatedAt?: Date | null;
  /** Distinguishes an explicit re-review of the same head (e.g. `manual:{comment id}`). */
  triggerSuffix?: string;
}

export type StartReviewResult =
  | {
      outcome: 'started' | 'duplicate';
      reviewRunId: string;
      superseded: string[];
      jobsCancelled: number;
    }
  | { outcome: 'stale_event' | 'not_found' | 'pr_not_open' };

interface LockedPullRequest {
  id: string;
  repository_id: string;
  provider_number: number;
  head_sha: string;
  state: string;
  provider_updated_at: Date | null;
  provider: string;
  provider_repo_id: string;
}

/** `pr-review:{provider}:{provider_repo_id}:{pr}:{head_sha}[:{suffix}]` (PRD §76). */
export function reviewIdempotencyKey(
  pr: Pick<LockedPullRequest, 'provider' | 'provider_repo_id' | 'provider_number'>,
  headSha: string,
  suffix?: string,
): string {
  const base = `pr-review:${pr.provider}:${pr.provider_repo_id}:${pr.provider_number}:${headSha}`;
  return suffix ? `${base}:${suffix}` : base;
}

/**
 * Supersession on a new head (SUP-001), the entry point of every review (webhook, manual
 * trigger, reconciler). One transaction, pull request row first (the same lock order as the
 * publish gate, SUP-003, so the two serialize without deadlocks):
 *
 *  1. lock the pull request (`lock_timeout` 2 s);
 *  2. refuse a stale event (provider `updated_at` older than stored, for a different head);
 *  3. move the head;
 *  4. supersede every non-terminal run of another head, pointing at the new run;
 *  5. cancel their queued jobs (running ones stop at their next stage boundary, SUP-002);
 *  6. insert the new run (idempotent by key) and enqueue its `pr-review` job with the same key.
 *
 * Any error rolls the whole transaction back: no partial supersession.
 */
@Injectable()
export class SupersessionService {
  private readonly logger = new Logger(SupersessionService.name);

  constructor(
    private readonly dbs: DbService,
    @Inject(REVIEW_JOBS) private readonly jobs: ReviewJobs,
  ) {}

  startReview(input: StartReviewInput): Promise<StartReviewResult> {
    return trace
      .getTracer(TRACER_NAME)
      .startActiveSpan(
        'supersession',
        { attributes: { pull_request_id: input.pullRequestId, commit_sha: input.headSha } },
        async (span) => {
          try {
            const result = await this.dbs.withTx(input.organizationId, (trx) =>
              this.startInTx(trx, input),
            );
            span.setAttribute('outcome', result.outcome);
            if ('reviewRunId' in result) span.setAttribute('review_run_id', result.reviewRunId);
            return result;
          } catch (err) {
            span.setStatus({ code: SpanStatusCode.ERROR });
            throw err;
          } finally {
            span.end();
          }
        },
      );
  }

  /** The same transaction body, for callers that already hold a tenant transaction. */
  async startInTx(trx: Tx, input: StartReviewInput): Promise<StartReviewResult> {
    await sql`select set_config('lock_timeout', ${SUPERSESSION_LOCK_TIMEOUT}, true)`.execute(trx);
    const pr = await this.lockPullRequest(trx, input.pullRequestId);
    if (!pr) return { outcome: 'not_found' };
    if (pr.state !== 'open') return { outcome: 'pr_not_open' };

    const at = input.prUpdatedAt ?? null;
    if (
      at &&
      pr.provider_updated_at &&
      at.getTime() < pr.provider_updated_at.getTime() &&
      input.headSha !== pr.head_sha
    ) {
      incCounter('supersession_stale_events_total');
      return { outcome: 'stale_event' };
    }

    await trx
      .updateTable('pull_requests')
      .set({
        head_sha: input.headSha,
        base_sha: input.baseSha,
        ...(at
          ? { provider_updated_at: sql<Date>`greatest(provider_updated_at, ${at}::timestamptz)` }
          : {}),
      })
      .where('id', '=', pr.id)
      .execute();

    const active = await trx
      .selectFrom('review_runs')
      .select(['id', 'head_sha'])
      .where('pull_request_id', '=', pr.id)
      .where('state', 'in', [...ACTIVE_RUN_STATES])
      .forUpdate()
      .execute();
    const activeSameHead = active.find((r) => r.head_sha === input.headSha);

    const baseKey = reviewIdempotencyKey(pr, input.headSha, input.triggerSuffix);
    let key = baseKey;
    let runId: string | undefined = activeSameHead?.id;
    let retryOf: string | null = null;
    let create = false;

    if (!runId) {
      const existing = await trx
        .selectFrom('review_runs')
        .select(['id', 'state'])
        .where('idempotency_key', '=', baseKey)
        .executeTakeFirst();
      const latestForHead = await trx
        .selectFrom('review_runs')
        .select(['id', 'state'])
        .where('pull_request_id', '=', pr.id)
        .where('head_sha', '=', input.headSha)
        .orderBy('created_at', 'desc')
        .orderBy('id', 'desc')
        .limit(1)
        .executeTakeFirst();
      if (!existing && !latestForHead) {
        create = true;
      } else if (!existing && latestForHead) {
        // An explicit re-review of a head that already has a run.
        create = true;
        retryOf = latestForHead.id;
      } else if (existing && existing.state !== 'COMPLETED' && latestForHead) {
        // The head came back after its run was superseded, cancelled or failed: review it again.
        // The key names the run it retries, so a repeated event is still a duplicate.
        key = `${baseKey}:rerun:${latestForHead.id}`;
        const rerun = await trx
          .selectFrom('review_runs')
          .select('id')
          .where('idempotency_key', '=', key)
          .executeTakeFirst();
        if (rerun) {
          runId = rerun.id;
        } else if (latestForHead.state === 'COMPLETED') {
          runId = latestForHead.id;
        } else {
          create = true;
          retryOf = latestForHead.id;
        }
      } else {
        runId = existing!.id;
      }
    }
    const newRunId = runId ?? uuidv7();

    // Older heads lose: SUPERSEDED, pointing at the run of the current head (checked at commit).
    const superseded = (
      await trx
        .updateTable('review_runs')
        .set({
          state: 'SUPERSEDED',
          superseded_by: newRunId,
          superseded_by_head: input.headSha,
          superseded_at: sql<Date>`now()`,
          completed_at: sql<Date>`now()`,
        })
        .where('pull_request_id', '=', pr.id)
        .where('head_sha', '<>', input.headSha)
        .where('state', 'in', [...ACTIVE_RUN_STATES])
        .returning('id')
        .execute()
    ).map((r) => r.id);
    const jobsCancelled =
      superseded.length > 0 ? await this.jobs.cancelQueuedForRuns(trx, superseded) : 0;
    if (superseded.length > 0) {
      incCounter('review_runs_superseded_total', {}, superseded.length);
      if (jobsCancelled > 0) {
        incCounter('jobs_cancelled_total', { reason: 'superseded' }, jobsCancelled);
      }
    }

    if (!create) {
      return { outcome: 'duplicate', reviewRunId: newRunId, superseded, jobsCancelled };
    }

    const inserted = await trx
      .insertInto('review_runs')
      .values({
        id: newRunId,
        organization_id: input.organizationId,
        repository_id: pr.repository_id,
        pull_request_id: pr.id,
        head_sha: input.headSha,
        base_sha: input.baseSha,
        trigger: input.trigger,
        depth: input.depth ?? 'standard',
        state: 'RECEIVED',
        idempotency_key: key,
        retry_of: retryOf,
      })
      .onConflict((oc) => oc.column('idempotency_key').doNothing())
      .returning('id')
      .executeTakeFirst();
    if (!inserted) {
      // Unreachable while the PR lock is held; kept so a duplicate never enqueues twice.
      const row = await trx
        .selectFrom('review_runs')
        .select('id')
        .where('idempotency_key', '=', key)
        .executeTakeFirstOrThrow();
      return { outcome: 'duplicate', reviewRunId: row.id, superseded, jobsCancelled };
    }
    await this.jobs.enqueuePrReview(trx, {
      reviewRunId: newRunId,
      organizationId: input.organizationId,
      repositoryId: pr.repository_id,
      idempotencyKey: key,
    });
    incCounter('review_runs_started_total', { trigger: input.trigger });
    this.logger.log(
      `review run started run=${newRunId} pr=${pr.id} superseded=${superseded.length}`,
    );
    return { outcome: 'started', reviewRunId: newRunId, superseded, jobsCancelled };
  }

  /**
   * Cancels every active run of a pull request (PR closed, `/review cancel`), cancelling their
   * queued jobs, with the same lock order. `closedState` also records the PR state.
   */
  cancelActiveRuns(
    organizationId: string,
    pullRequestId: string,
    opts: { closedState?: 'closed' | 'merged' } = {},
  ): Promise<string[]> {
    return this.dbs.withTx(organizationId, async (trx) => {
      await sql`select set_config('lock_timeout', ${SUPERSESSION_LOCK_TIMEOUT}, true)`.execute(trx);
      const pr = await this.lockPullRequest(trx, pullRequestId);
      if (!pr) return [];
      if (opts.closedState) {
        await trx
          .updateTable('pull_requests')
          .set({ state: opts.closedState })
          .where('id', '=', pr.id)
          .execute();
      }
      const cancelled = (
        await trx
          .updateTable('review_runs')
          .set({ state: 'CANCELLED', completed_at: sql<Date>`now()` })
          .where('pull_request_id', '=', pr.id)
          .where('state', 'in', [...ACTIVE_RUN_STATES])
          .returning('id')
          .execute()
      ).map((r) => r.id);
      if (cancelled.length > 0) {
        const jobs = await this.jobs.cancelQueuedForRuns(trx, cancelled);
        if (jobs > 0) incCounter('jobs_cancelled_total', { reason: 'cancelled' }, jobs);
      }
      return cancelled;
    });
  }

  private async lockPullRequest(trx: Tx, pullRequestId: string): Promise<LockedPullRequest | null> {
    const row = await trx
      .selectFrom('pull_requests as pr')
      .innerJoin('repositories as r', 'r.id', 'pr.repository_id')
      .select([
        'pr.id',
        'pr.repository_id',
        'pr.provider_number',
        'pr.head_sha',
        'pr.state',
        'pr.provider_updated_at',
        'r.provider',
        'r.provider_repo_id',
      ])
      .where('pr.id', '=', pullRequestId)
      .forUpdate('pr')
      .executeTakeFirst();
    return row ?? null;
  }
}
