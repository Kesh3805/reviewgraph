import { Inject, Injectable, Logger, Optional } from '@nestjs/common';
import { incCounter } from '../common/metrics';
import { DbService } from '../db/db.module';
import { PROVIDER_RESOLVER, type ProviderResolver } from '../providers/ports';
import type { PublishableFinding, PublishContext } from './publish-source';
import type { PreviouslyReportedFinding } from './render/summary';
import type { Severity } from './render/types';

/**
 * Symbol lineage between two commits (SID-005): maps old symbol keys to their current keys
 * (renames, moves). The default knows no lineage, so only identical symbols match.
 */
export interface SymbolLineage {
  map(
    ctx: { organizationId: string; repositoryId: string },
    symbols: string[],
    fromSha: string,
    toSha: string,
  ): Promise<Map<string, string>>;
}
export const SYMBOL_LINEAGE = Symbol('SYMBOL_LINEAGE');

/** A finding published by an earlier run of the same PR that is still being tracked. */
export interface PreviousFinding {
  publishedId: string;
  providerCommentId: string | null;
  path: string | null;
  line: number | null;
  headSha: string;
  fingerprint: string;
  category: string;
  severity: Severity;
  title: string;
  affectedSymbols: string[];
}

export interface StalePlan {
  /** Current findings already reported earlier: not reposted (`carried_over`). */
  carriedOver: Map<string, PreviousFinding[]>;
  /** Absent now and their code changed: their threads are resolved. */
  fixed: PreviousFinding[];
  /** Absent now but their code did not change: left open and listed in the summary. */
  unknown: PreviousFinding[];
}

export const emptyStalePlan = (): StalePlan => ({ carriedOver: new Map(), fixed: [], unknown: [] });

/**
 * Pure matching: a previous finding is "still present" when a current finding has the same
 * root-cause fingerprint, or the same category and a symbol that the lineage maps onto one of the
 * current finding's symbols. `changedPaths(headSha)` returns the paths changed since that head
 * (null when unknown): an absent finding whose path changed is fixed, otherwise unknown.
 */
export function classifyStale(
  previous: PreviousFinding[],
  current: PublishableFinding[],
  lineage: Map<string, string>,
  changedPaths: (fromSha: string) => Set<string> | null,
): StalePlan {
  const plan = emptyStalePlan();
  for (const prev of previous) {
    const mapped = prev.affectedSymbols.map((s) => lineage.get(s) ?? s);
    const match = current.find(
      (f) =>
        f.fingerprint === prev.fingerprint ||
        (f.category === prev.category &&
          mapped.length > 0 &&
          f.affectedSymbols.some((s) => mapped.includes(s))),
    );
    if (match) {
      const list = plan.carriedOver.get(match.fingerprint) ?? [];
      list.push(prev);
      plan.carriedOver.set(match.fingerprint, list);
      continue;
    }
    const changed = changedPaths(prev.headSha);
    if (changed && (prev.path === null || changed.has(prev.path))) plan.fixed.push(prev);
    else plan.unknown.push(prev);
  }
  return plan;
}

export function previouslyReported(plan: StalePlan): PreviouslyReportedFinding[] {
  return plan.unknown.map((p) => ({
    severity: p.severity,
    title: p.title,
    ...(p.path ? { path: p.path } : {}),
    ...(p.line ? { line: p.line } : {}),
  }));
}

/**
 * Stale comment resolution on re-review (GH-011). Before posting a new head's review it
 * classifies the PR's earlier, still-tracked findings; after a successful post it records the
 * outcome on `published_findings` and resolves the App's own threads of fixed findings. Human
 * threads are never touched and nothing is ever deleted. A provider failure is logged and
 * counted, never fails the publish; the next run retries (status updates are monotonic).
 */
@Injectable()
export class StaleResolutionService {
  private readonly logger = new Logger(StaleResolutionService.name);

  constructor(
    private readonly dbs: DbService,
    @Inject(PROVIDER_RESOLVER) private readonly providers: ProviderResolver,
    @Optional() @Inject(SYMBOL_LINEAGE) private readonly lineage?: SymbolLineage,
  ) {}

  async plan(ctx: PublishContext, current: PublishableFinding[]): Promise<StalePlan> {
    const previous = await this.previous(ctx);
    if (previous.length === 0) return emptyStalePlan();

    let lineage = new Map<string, string>();
    if (this.lineage) {
      const symbols = [...new Set(previous.flatMap((p) => p.affectedSymbols))];
      const heads = [...new Set(previous.map((p) => p.headSha))];
      try {
        for (const head of heads) {
          const part = await this.lineage.map(
            { organizationId: ctx.organizationId, repositoryId: ctx.repositoryId },
            symbols,
            head,
            ctx.headSha,
          );
          lineage = new Map([...lineage, ...part]);
        }
      } catch {
        this.logger.warn(`symbol lineage unavailable run=${ctx.runId}`);
      }
    }

    const provider = this.providers.repository(ctx.pr.provider);
    const changed = new Map<string, Set<string> | null>();
    for (const head of new Set(previous.map((p) => p.headSha))) {
      if (head === ctx.headSha || !provider.listFilesBetween) {
        changed.set(head, null);
        continue;
      }
      try {
        changed.set(head, new Set(await provider.listFilesBetween(ctx.pr, head, ctx.headSha)));
      } catch {
        // Without the comparison nothing can be called fixed: unknown is the safe answer.
        changed.set(head, null);
      }
    }
    return classifyStale(previous, current, lineage, (head) => changed.get(head) ?? null);
  }

  /** Records the plan after the new review was posted and resolves fixed threads. */
  async apply(ctx: PublishContext, plan: StalePlan): Promise<void> {
    const carried = [...plan.carriedOver.values()].flat().map((p) => p.publishedId);
    let resolvedIds = plan.fixed.filter((p) => !p.providerCommentId).map((p) => p.publishedId);
    const threadComments = plan.fixed
      .filter((p) => p.providerCommentId)
      .map((p) => p.providerCommentId!);
    if (threadComments.length > 0) {
      try {
        const result = await this.providers
          .publisher(ctx.pr.provider)
          .resolveThreads(ctx.pr, threadComments);
        const resolved = new Set(result.resolved);
        resolvedIds = resolvedIds.concat(
          plan.fixed
            .filter((p) => p.providerCommentId && resolved.has(p.providerCommentId))
            .map((p) => p.publishedId),
        );
        if (result.resolved.length > 0) {
          incCounter('stale_threads_resolved_total', {}, result.resolved.length);
        }
      } catch (err) {
        incCounter('stale_resolution_failures_total');
        this.logger.warn(
          `stale thread resolution failed run=${ctx.runId} (${err instanceof Error ? err.name : 'error'})`,
        );
      }
    }
    const unknown = plan.unknown.map((p) => p.publishedId);

    await this.dbs.withTx(ctx.organizationId, async (trx) => {
      if (carried.length > 0) {
        await trx
          .updateTable('published_findings')
          .set({ status: 'carried_over' })
          .where('id', 'in', carried)
          .where('status', '<>', 'resolved')
          .execute();
      }
      if (resolvedIds.length > 0) {
        await trx
          .updateTable('published_findings')
          .set({ status: 'resolved', resolved_in_run_id: ctx.runId })
          .where('id', 'in', resolvedIds)
          .execute();
      }
      if (unknown.length > 0) {
        await trx
          .updateTable('published_findings')
          .set({ status: 'unknown' })
          .where('id', 'in', unknown)
          .where('status', '<>', 'resolved')
          .execute();
      }
    });
    if (carried.length > 0) incCounter('findings_carried_over_total', {}, carried.length);
  }

  private previous(ctx: PublishContext): Promise<PreviousFinding[]> {
    return this.dbs.withTx(ctx.organizationId, async (trx) => {
      const rows = await trx
        .selectFrom('published_findings as pf')
        .innerJoin('verified_findings as vf', 'vf.id', 'pf.verified_finding_id')
        .innerJoin('candidate_findings as cf', 'cf.id', 'vf.candidate_finding_id')
        .select([
          'pf.id',
          'pf.provider_comment_id',
          'pf.path',
          'pf.start_line',
          'pf.end_line',
          'pf.head_sha',
          'cf.fingerprint',
          'cf.category',
          'cf.title',
          'cf.affected_symbols',
          'vf.severity',
        ])
        .where('pf.pull_request_id', '=', ctx.pullRequestId)
        .where('pf.review_run_id', '<>', ctx.runId)
        .where('pf.status', 'in', ['open', 'carried_over', 'unknown'])
        .orderBy('pf.published_at')
        .execute();
      return rows.map((r) => ({
        publishedId: r.id,
        providerCommentId: r.provider_comment_id,
        path: r.path,
        line: r.end_line ?? r.start_line,
        headSha: r.head_sha,
        fingerprint: r.fingerprint,
        category: r.category,
        severity: r.severity as Severity,
        title: r.title,
        affectedSymbols: r.affected_symbols,
      }));
    });
  }
}
