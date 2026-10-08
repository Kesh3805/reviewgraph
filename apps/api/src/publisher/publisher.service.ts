import { Inject, Injectable, Logger } from '@nestjs/common';
import { SpanStatusCode, trace } from '@opentelemetry/api';
import { sql } from 'kysely';
import { incCounter } from '../common/metrics';
import { DbService } from '../db/db.module';
import type { Tx } from '../db/tx';
import {
  PROVIDER_RESOLVER,
  ProviderError,
  collectChangedFiles,
  type CheckRunConclusion,
  type ExistingReview,
  type InlineReviewComment,
  type PublishRequest,
  type ProviderResolver,
  type PublishedComment,
} from '../providers/ports';
import { TRACER_NAME } from '../telemetry/tracer.service';
import { PublishGate, type GateSkipReason } from './publish-gate';
import { PublishSource, type PublishableFinding, type PublishContext } from './publish-source';
import { DiffIndex } from './render/diff-index';
import { planPublication, type PublicationPlan } from './render/plan';
import { renderSummary, type SummaryInput } from './render/summary';
import type { Severity } from './render/types';
import {
  StaleResolutionService,
  emptyStalePlan,
  previouslyReported,
  type StalePlan,
} from './stale-resolution.service';

export const CHECK_RUN_NAME = 'ReviewGraph';

export type PublishOutcome =
  | 'published'
  | 'adopted'
  | 'already_published'
  | 'skipped_superseded'
  | 'failed_permanent'
  | 'not_found';

/** `reviewgraph:run=<id>`: the hidden marker that finds an already-posted review on retry. */
export const runMarker = (runId: string): string => `reviewgraph:run=${runId}`;

/**
 * The check run never blocks a merge and is never `success` unless the run published cleanly
 * with complete coverage (INV-012).
 */
export function checkRunResult(input: { failed: boolean; degraded: boolean; findings: number }): {
  conclusion: CheckRunConclusion;
  title: string;
} {
  if (input.failed) return { conclusion: 'neutral', title: 'Review could not be published' };
  if (input.degraded) return { conclusion: 'neutral', title: 'Review incomplete' };
  if (input.findings > 0) {
    return {
      conclusion: 'neutral',
      title: `${input.findings} finding${input.findings === 1 ? '' : 's'} published`,
    };
  }
  return { conclusion: 'success', title: 'No findings' };
}

interface Prepared {
  findings: PublishableFinding[];
  plan: PublicationPlan;
  stale: StalePlan;
  summary: SummaryInput;
}

type GateOutcome =
  | { kind: 'skipped'; reason: GateSkipReason }
  | {
      kind: 'posted';
      reviewId: string;
      comments: PublishedComment[];
      adopted: boolean;
      plan: PublicationPlan;
    }
  | { kind: 'failed'; message: string };

/**
 * The publisher (GH-009): one atomic `COMMENT` review per run, posted while the publish gate
 * (SUP-003) holds the pull request and run locks, so nothing is ever posted for an obsolete
 * head. Before posting it always looks for a review carrying the run marker and adopts it, so a
 * retry after a crash (or a timeout after GitHub accepted the POST) never posts twice. After
 * the post it records `published_findings`, completes the run, applies stale resolution
 * (GH-011) and upserts the check run.
 */
@Injectable()
export class PublisherService {
  private readonly logger = new Logger(PublisherService.name);

  constructor(
    private readonly dbs: DbService,
    private readonly gate: PublishGate,
    private readonly source: PublishSource,
    private readonly stale: StaleResolutionService,
    @Inject(PROVIDER_RESOLVER) private readonly providers: ProviderResolver,
  ) {}

  publish(reviewRunId: string): Promise<PublishOutcome> {
    return trace
      .getTracer(TRACER_NAME)
      .startActiveSpan(
        'publication',
        { attributes: { review_run_id: reviewRunId } },
        async (span) => {
          try {
            const outcome = await this.run(reviewRunId);
            span.setAttribute('outcome', outcome);
            incCounter('publish_outcomes_total', { outcome });
            return outcome;
          } catch (err) {
            span.setStatus({ code: SpanStatusCode.ERROR });
            incCounter('publish_outcomes_total', { outcome: 'retry' });
            throw err;
          } finally {
            span.end();
          }
        },
      );
  }

  private async run(reviewRunId: string): Promise<PublishOutcome> {
    const org = await this.organizationOf(reviewRunId);
    if (!org) return 'not_found';
    const ctx = await this.dbs.withTx(org, (trx) => this.source.context(trx, reviewRunId));
    if (!ctx) return 'not_found';

    // A previous attempt posted and committed: only the follow-up steps remain.
    if (ctx.publication?.state === 'posted' && ctx.publication.providerReviewId) {
      const existing = await this.publisher(ctx).findExistingReview(ctx.pr, runMarker(ctx.runId));
      const prepared = await this.prepare(ctx);
      await this.finalize(ctx, prepared, prepared.plan, existing?.comments ?? []);
      return 'already_published';
    }
    if (ctx.publication?.state === 'skipped' || ctx.publication?.state === 'failed') {
      return ctx.publication.state === 'skipped' ? 'skipped_superseded' : 'failed_permanent';
    }

    const prepared = await this.prepare(ctx);
    const outcome = await this.dbs.withTx(org, (trx) => this.gateAndPost(trx, ctx, prepared));

    switch (outcome.kind) {
      case 'skipped':
        this.logger.log(`publish skipped run=${ctx.runId} reason=${outcome.reason}`);
        return 'skipped_superseded';
      case 'failed':
        await this.upsertCheckRun(
          ctx,
          { failed: true, degraded: false, findings: 0 },
          outcome.message,
        );
        return 'failed_permanent';
      case 'posted':
        await this.finalize(ctx, prepared, outcome.plan, outcome.comments);
        return outcome.adopted ? 'adopted' : 'published';
    }
  }

  /** Rendering and provider reads happen before the gate, so the lock is held for one POST. */
  private async prepare(ctx: PublishContext): Promise<Prepared> {
    const findings = await this.dbs.withTx(ctx.organizationId, (trx) =>
      this.source.findings(trx, ctx),
    );
    const { files } = await collectChangedFiles(
      this.providers.repository(ctx.pr.provider).listChangedFiles(ctx.pr),
    );
    let stale = emptyStalePlan();
    try {
      stale = await this.stale.plan(ctx, findings);
    } catch (err) {
      incCounter('stale_resolution_failures_total');
      this.logger.warn(`stale classification failed run=${ctx.runId} (${errorName(err)})`);
    }
    const postable = findings.filter((f) => !stale.carriedOver.has(f.fingerprint));
    const plan = planPublication(postable, DiffIndex.build(files));
    const base = await this.dbs.withTx(ctx.organizationId, (trx) =>
      this.source.summaryBase(trx, ctx, files.length),
    );
    return { findings, plan, stale, summary: this.summaryOf(base, plan, stale) };
  }

  private summaryOf(
    base: Omit<SummaryInput, 'findingsBySeverity' | 'outsideDiff' | 'previouslyReported'>,
    plan: PublicationPlan,
    stale: StalePlan,
  ): SummaryInput {
    const bySeverity: Partial<Record<Severity, number>> = {};
    for (const f of [
      ...plan.inline.map((p) => p.finding),
      ...plan.relocated.map((r) => r.finding),
    ]) {
      bySeverity[f.severity] = (bySeverity[f.severity] ?? 0) + 1;
    }
    return {
      ...base,
      findingsBySeverity: bySeverity,
      outsideDiff: plan.relocated.map((r) => ({
        severity: r.finding.severity,
        title: r.finding.title,
        ...(r.finding.location
          ? { path: r.finding.location.path, line: r.finding.location.startLine }
          : {}),
        reason: r.reason,
      })),
      previouslyReported: previouslyReported(stale),
    };
  }

  private async gateAndPost(
    trx: Tx,
    ctx: PublishContext,
    prepared: Prepared,
  ): Promise<GateOutcome> {
    const gate = await this.gate.acquire(trx, ctx.runId);
    if (!gate.proceed) {
      // The head moved, the PR closed or access was lost: the run ends without a review. A run
      // already SUPERSEDED or CANCELLED keeps its state.
      if (gate.run?.state === 'PUBLISHING') {
        await trx
          .updateTable('review_runs')
          .set({ state: 'CANCELLED', completed_at: sql<Date>`now()` })
          .where('id', '=', ctx.runId)
          .where('state', '=', 'PUBLISHING')
          .execute();
      }
      if (gate.reason !== 'not_found' && gate.reason !== 'not_publishing') {
        await this.recordPublication(trx, ctx, 'skipped', null, gate.reason);
      }
      return { kind: 'skipped', reason: gate.reason };
    }

    await trx
      .insertInto('publications')
      .values({ review_run_id: ctx.runId, organization_id: ctx.organizationId, state: 'posting' })
      .onConflict((oc) =>
        oc.column('review_run_id').doUpdateSet({ attempt: sql<number>`publications.attempt + 1` }),
      )
      .execute();

    const publisher = this.publisher(ctx);
    const marker = runMarker(ctx.runId);
    try {
      // Always look first: a crash after GitHub accepted the POST rolls our row back, but the
      // review (and its marker) exists, and it must be adopted rather than posted again.
      const existing: ExistingReview | null = await publisher.findExistingReview(ctx.pr, marker);
      if (existing) {
        await this.recordPublication(trx, ctx, 'posted', existing.providerReviewId, 'adopted');
        return {
          kind: 'posted',
          reviewId: existing.providerReviewId,
          comments: existing.comments,
          adopted: true,
          plan: prepared.plan,
        };
      }
      let plan = prepared.plan;
      let result;
      try {
        result = await publisher.publishReview(this.request(ctx, plan, prepared));
      } catch (err) {
        // 422 "line must be part of the diff": every inline comment moves to the summary and
        // the post is retried once (INV-015, nothing is dropped).
        if (!(err instanceof ProviderError) || err.kind !== 'invalid' || plan.inline.length === 0) {
          throw err;
        }
        incCounter('publish_inline_relocated_total', {}, plan.inline.length);
        plan = {
          inline: [],
          relocated: [
            ...plan.relocated,
            ...plan.inline.map((p) => ({ finding: p.finding, reason: 'outside_diff' as const })),
          ],
        };
        result = await publisher.publishReview(this.request(ctx, plan, prepared));
      }
      await this.recordPublication(trx, ctx, 'posted', result.providerReviewId, 'published');
      return {
        kind: 'posted',
        reviewId: result.providerReviewId,
        comments: result.comments,
        adopted: false,
        plan,
      };
    } catch (err) {
      if (err instanceof ProviderError && !err.retryable) {
        const message = err.message.slice(0, 2000);
        await trx
          .updateTable('review_runs')
          .set({
            state: 'FAILED_PUBLISH',
            failure_class: 'permanent',
            failure_detail: message,
            completed_at: sql<Date>`now()`,
          })
          .where('id', '=', ctx.runId)
          .where('state', '=', 'PUBLISHING')
          .execute();
        await this.recordPublication(trx, ctx, 'failed', null, err.kind);
        this.logger.warn(`publish failed permanently run=${ctx.runId} kind=${err.kind}`);
        return { kind: 'failed', message };
      }
      // Transient or rate limited: roll back and let the job retry with backoff.
      throw err;
    }
  }

  private request(ctx: PublishContext, plan: PublicationPlan, prepared: Prepared): PublishRequest {
    const summary = renderSummary(this.summaryOf(prepared.summary, plan, prepared.stale));
    const comments: InlineReviewComment[] = plan.inline.map((p) => ({
      path: p.anchor.path,
      line: p.anchor.line,
      side: p.anchor.side,
      ...(p.anchor.startLine !== undefined
        ? { startLine: p.anchor.startLine, startSide: p.anchor.startSide ?? p.anchor.side }
        : {}),
      body: p.body,
      findingId: p.finding.id,
    }));
    return {
      pr: ctx.pr,
      headSha: ctx.headSha,
      event: 'COMMENT',
      summary,
      comments,
      marker: runMarker(ctx.runId),
    };
  }

  private async recordPublication(
    trx: Tx,
    ctx: PublishContext,
    state: 'posted' | 'skipped' | 'failed',
    providerReviewId: string | null,
    outcome: string,
  ): Promise<void> {
    await trx
      .insertInto('publications')
      .values({
        review_run_id: ctx.runId,
        organization_id: ctx.organizationId,
        state,
        provider_review_id: providerReviewId,
        outcome,
        posted_at: state === 'posted' ? sql<Date>`now()` : null,
      })
      .onConflict((oc) =>
        oc.column('review_run_id').doUpdateSet({
          state,
          provider_review_id: providerReviewId,
          outcome,
          posted_at: state === 'posted' ? sql<Date>`now()` : null,
        }),
      )
      .execute();
    if (state === 'posted') {
      // Completed while the gate locks are still held: a supersession that was waiting for the
      // post then finds no active run to supersede (a posted review's run never ends SUPERSEDED).
      await trx
        .updateTable('review_runs')
        .set({ state: 'COMPLETED', completed_at: sql<Date>`now()` })
        .where('id', '=', ctx.runId)
        .where('state', '=', 'PUBLISHING')
        .execute();
    }
  }

  /** Steps after the post, all idempotent: safe to repeat on a retry. */
  private async finalize(
    ctx: PublishContext,
    prepared: Prepared,
    plan: PublicationPlan,
    comments: PublishedComment[],
  ): Promise<void> {
    const reviewId = await this.dbs.withTx(ctx.organizationId, async (trx) => {
      const pub = await trx
        .selectFrom('publications')
        .select('provider_review_id')
        .where('review_run_id', '=', ctx.runId)
        .executeTakeFirst();
      return pub?.provider_review_id ?? null;
    });
    const byFinding = new Map<string, string>();
    for (const c of comments) byFinding.set(c.findingId, c.providerCommentId);

    const rows = [
      ...plan.inline.map((p) => {
        const commentId = byFinding.get(p.finding.id) ?? byFinding.get(p.finding.fingerprint);
        return {
          finding: p.finding as PublishableFinding,
          placement: 'inline' as const,
          path: p.anchor.path,
          startLine: p.anchor.startLine ?? p.anchor.line,
          endLine: p.anchor.line,
          side: p.anchor.side,
          commentId: commentId ?? null,
        };
      }),
      ...plan.relocated.map((r) => ({
        finding: r.finding as PublishableFinding,
        placement: 'summary' as const,
        path: null,
        startLine: null,
        endLine: null,
        side: null,
        commentId: null,
      })),
    ];

    await this.dbs.withTx(ctx.organizationId, async (trx) => {
      if (rows.length > 0) {
        await trx
          .insertInto('published_findings')
          .values(
            rows.map((r) => ({
              organization_id: ctx.organizationId,
              verified_finding_id: r.finding.id,
              review_run_id: ctx.runId,
              pull_request_id: ctx.pullRequestId,
              provider: ctx.pr.provider,
              // An inline comment GitHub did not echo back is tracked as a summary finding.
              placement: r.placement === 'inline' && r.commentId ? 'inline' : 'summary',
              path: r.placement === 'inline' && r.commentId ? r.path : null,
              start_line: r.placement === 'inline' && r.commentId ? r.startLine : null,
              end_line: r.placement === 'inline' && r.commentId ? r.endLine : null,
              side: r.placement === 'inline' && r.commentId ? r.side : null,
              head_sha: ctx.headSha,
              provider_review_id: reviewId,
              provider_comment_id: r.commentId,
              published_at: sql<Date>`now()`,
            })),
          )
          .onConflict((oc) => oc.doNothing())
          .execute();
        await trx
          .updateTable('candidate_findings')
          .set({ state: 'PUBLISHED' })
          .where(
            'id',
            'in',
            rows.map((r) => r.finding.candidateId),
          )
          .where('state', '=', 'PRIORITIZED')
          .execute();
      }
      await trx
        .updateTable('review_runs')
        .set({ state: 'COMPLETED', completed_at: sql<Date>`now()` })
        .where('id', '=', ctx.runId)
        .where('state', '=', 'PUBLISHING')
        .execute();
    });
    for (const r of rows) {
      incCounter('published_findings_total', {
        severity: r.finding.severity,
        reviewer: r.finding.reviewer,
      });
    }

    try {
      await this.stale.apply(ctx, prepared.stale);
    } catch (err) {
      incCounter('stale_resolution_failures_total');
      this.logger.warn(`stale resolution failed run=${ctx.runId} (${errorName(err)})`);
    }
    await this.upsertCheckRun(ctx, {
      failed: false,
      degraded: prepared.summary.degraded,
      findings: rows.length,
    });
  }

  private async upsertCheckRun(
    ctx: PublishContext,
    input: { failed: boolean; degraded: boolean; findings: number },
    detail?: string,
  ): Promise<void> {
    const { conclusion, title } = checkRunResult(input);
    const summary = detail
      ? `The review for ${ctx.headSha.slice(0, 7)} could not be published.`
      : `Review of ${ctx.headSha.slice(0, 7)}: ${title}.`;
    const existing = await this.dbs.withTx(ctx.organizationId, (trx) =>
      trx
        .selectFrom('check_runs')
        .select('provider_check_run_id')
        .where('review_run_id', '=', ctx.runId)
        .executeTakeFirst(),
    );
    const { checkRunId } = await this.publisher(ctx).upsertCheckRun({
      repo: ctx.pr,
      headSha: ctx.headSha,
      name: CHECK_RUN_NAME,
      status: 'completed',
      conclusion,
      title,
      summary,
      externalId: ctx.runId,
      ...(existing ? { checkRunId: existing.provider_check_run_id } : {}),
    });
    await this.dbs.withTx(ctx.organizationId, (trx) =>
      trx
        .insertInto('check_runs')
        .values({
          review_run_id: ctx.runId,
          organization_id: ctx.organizationId,
          provider_check_run_id: checkRunId,
          conclusion,
          title,
        })
        .onConflict((oc) =>
          oc.column('review_run_id').doUpdateSet({
            provider_check_run_id: checkRunId,
            conclusion,
            title,
          }),
        )
        .execute(),
    );
  }

  private publisher(ctx: PublishContext) {
    return this.providers.publisher(ctx.pr.provider);
  }

  private async organizationOf(runId: string): Promise<string | null> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ org: string | null }>`select resolve_org('review', ${runId}::uuid) as org`.execute(trx),
    );
    return rows[0]?.org ?? null;
  }
}

function errorName(err: unknown): string {
  return err instanceof Error ? err.name : 'error';
}
