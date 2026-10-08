import { Injectable } from '@nestjs/common';
import { sql } from 'kysely';
import type { Tx } from '../db/tx';
import type { ProviderKind, PrRef } from '../providers/ports';
import type { SummaryInput, SuppressionReason } from './render/summary';
import type { EvidenceItem, RenderableFinding, Severity } from './render/types';

/** A verified, prioritized finding ready to render, plus what stale resolution matches on. */
export interface PublishableFinding extends RenderableFinding {
  /** `verified_findings.id`. */
  id: string;
  candidateId: string;
  category: string;
  affectedSymbols: string[];
}

/** Everything about a run the publisher needs, read without locks before the gate. */
export interface PublishContext {
  organizationId: string;
  runId: string;
  runState: string;
  headSha: string;
  repositoryId: string;
  pullRequestId: string;
  pr: PrRef;
  degradedReviewers: string[];
  publication: { state: string; attempt: number; providerReviewId: string | null } | null;
  checkRunId: string | null;
}

const SUPPRESSION_BY_STATE: Record<string, SuppressionReason> = {
  SUPPRESSED_LOW_CONFIDENCE: 'low_confidence',
  SUPPRESSED_DUPLICATE: 'duplicate',
  SUPPRESSED_PREEXISTING: 'pre_existing',
  SUPPRESSED_NOT_ACTIONABLE: 'not_actionable',
  SUPPRESSED_POLICY: 'policy',
};

const SEVERITIES: readonly Severity[] = ['info', 'low', 'medium', 'high', 'critical'];

interface Explanation {
  what_changed?: unknown;
  why_risky?: unknown;
  behavior_result?: unknown;
  corrective_direction?: unknown;
  evidence_path?: unknown;
  latent?: unknown;
}

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);

/**
 * Maps the stored verification evidence onto the renderer's view model. The verifier records
 * the PRD §59 explanation as `evidence.explanation` (`what_changed`, `why_risky`,
 * `behavior_result`, `corrective_direction`, `evidence_path`, `latent`) and cited items as an
 * array (`evidence.items` or the evidence itself). A finding without a complete explanation is
 * still published: the renderer relocates it to the summary (`render_error`), never drops it.
 */
export function explanationOf(
  evidence: unknown,
  description: string,
): Pick<
  RenderableFinding,
  | 'whatChanged'
  | 'whyRisky'
  | 'behaviorResult'
  | 'correctiveDirection'
  | 'evidencePath'
  | 'evidenceItems'
  | 'latent'
> {
  const obj =
    evidence && typeof evidence === 'object' && !Array.isArray(evidence)
      ? (evidence as { explanation?: Explanation; items?: unknown })
      : undefined;
  const ex: Explanation = obj?.explanation ?? {};
  const rawItems = Array.isArray(evidence) ? evidence : Array.isArray(obj?.items) ? obj.items : [];
  const evidenceItems: EvidenceItem[] = rawItems.flatMap((i: unknown) => {
    const item = i as { claim?: unknown; path?: unknown; line?: unknown };
    const claim = str(item?.claim);
    if (!claim) return [];
    return [
      {
        claim,
        ...(str(item.path) ? { path: str(item.path) } : {}),
        ...(typeof item.line === 'number' ? { line: item.line } : {}),
      },
    ];
  });
  const path = Array.isArray(ex.evidence_path)
    ? ex.evidence_path.filter((p): p is string => typeof p === 'string')
    : undefined;
  return {
    whatChanged: str(ex.what_changed) ?? description,
    whyRisky: str(ex.why_risky) ?? '',
    behaviorResult: str(ex.behavior_result) ?? '',
    correctiveDirection: str(ex.corrective_direction) ?? '',
    ...(path && path.length > 0 ? { evidencePath: path } : {}),
    ...(evidenceItems.length > 0 ? { evidenceItems } : {}),
    ...(ex.latent === true ? { latent: true } : {}),
  };
}

/** Reads runs, findings and summary counts for publication (GH-009). Pure SQL, tenant scoped. */
@Injectable()
export class PublishSource {
  async context(trx: Tx, runId: string): Promise<PublishContext | null> {
    const row = await trx
      .selectFrom('review_runs as rr')
      .innerJoin('pull_requests as pr', 'pr.id', 'rr.pull_request_id')
      .innerJoin('repositories as r', 'r.id', 'rr.repository_id')
      .innerJoin('provider_installations as i', 'i.id', 'r.installation_id')
      .leftJoin('publications as p', 'p.review_run_id', 'rr.id')
      .leftJoin('check_runs as c', 'c.review_run_id', 'rr.id')
      .select([
        'rr.id',
        'rr.organization_id',
        'rr.state',
        'rr.head_sha',
        'rr.repository_id',
        'rr.pull_request_id',
        'rr.degraded_reviewers',
        'pr.provider_number',
        'r.provider',
        'r.full_name',
        'i.provider_installation_id',
        'p.state as publication_state',
        'p.attempt as publication_attempt',
        'p.provider_review_id',
        'c.provider_check_run_id',
      ])
      .where('rr.id', '=', runId)
      .executeTakeFirst();
    if (!row) return null;
    const slash = row.full_name.indexOf('/');
    return {
      organizationId: row.organization_id,
      runId: row.id,
      runState: row.state,
      headSha: row.head_sha,
      repositoryId: row.repository_id,
      pullRequestId: row.pull_request_id,
      pr: {
        provider: row.provider as ProviderKind,
        installationId: String(row.provider_installation_id),
        owner: row.full_name.slice(0, slash),
        name: row.full_name.slice(slash + 1),
        number: row.provider_number,
      },
      degradedReviewers: row.degraded_reviewers,
      publication: row.publication_state
        ? {
            state: row.publication_state,
            attempt: row.publication_attempt ?? 1,
            providerReviewId: row.provider_review_id,
          }
        : null,
      checkRunId: row.provider_check_run_id,
    };
  }

  /** Verified findings that pass their publication band, in priority order (DED-004). */
  async findings(trx: Tx, ctx: PublishContext): Promise<PublishableFinding[]> {
    const rows = await trx
      .selectFrom('verified_findings as vf')
      .innerJoin('candidate_findings as cf', 'cf.id', 'vf.candidate_finding_id')
      .select([
        'vf.id',
        'vf.severity',
        'vf.computed_confidence',
        'vf.evidence',
        'cf.id as candidate_id',
        'cf.title',
        'cf.description',
        'cf.changed_path',
        'cf.changed_side',
        'cf.changed_start_line',
        'cf.changed_end_line',
        'cf.fingerprint',
        'cf.reviewer',
        'cf.category',
        'cf.affected_symbols',
      ])
      .where('vf.review_run_id', '=', ctx.runId)
      .where('cf.state', 'in', ['PRIORITIZED', 'PUBLISHED'])
      .where((eb) =>
        eb.or([
          eb('vf.band', '=', 'publish'),
          eb.and([
            eb('vf.band', '=', 'publish_if_medium_or_above'),
            eb('vf.severity', 'in', ['medium', 'high', 'critical']),
          ]),
        ]),
      )
      .orderBy(sql`vf.priority_score desc nulls last`)
      .orderBy('vf.id')
      .execute();
    return rows.map((r, i) => ({
      id: r.id,
      candidateId: r.candidate_id,
      shortId: `RG-${ctx.pr.number}-${String(i + 1).padStart(3, '0')}`,
      fingerprint: r.fingerprint,
      reviewRunId: ctx.runId,
      severity: SEVERITIES.includes(r.severity as Severity) ? (r.severity as Severity) : 'info',
      title: r.title,
      ...explanationOf(r.evidence, r.description),
      reviewer: r.reviewer,
      confidence: r.computed_confidence,
      location: {
        path: r.changed_path,
        startLine: r.changed_start_line,
        endLine: r.changed_end_line,
        side: r.changed_side === 'base' ? 'base' : 'head',
      },
      category: r.category,
      affectedSymbols: r.affected_symbols,
    }));
  }

  /** The counts and coverage of the summary; findings and relocations are filled by the caller. */
  async summaryBase(
    trx: Tx,
    ctx: PublishContext,
    changedFiles: number,
  ): Promise<Omit<SummaryInput, 'findingsBySeverity' | 'outsideDiff' | 'previouslyReported'>> {
    const states = await trx
      .selectFrom('candidate_findings')
      .select(['state', (eb) => eb.fn.countAll<string>().as('n')])
      .where('review_run_id', '=', ctx.runId)
      .groupBy('state')
      .execute();
    const verified = await trx
      .selectFrom('verified_findings')
      .select((eb) => eb.fn.countAll<string>().as('n'))
      .where('review_run_id', '=', ctx.runId)
      .executeTakeFirst();
    const reviewers = await trx
      .selectFrom('reviewer_runs')
      .select(['reviewer', 'state', 'error_class'])
      .where('review_run_id', '=', ctx.runId)
      .execute();

    const suppressed: Partial<Record<SuppressionReason, number>> = {};
    let candidates = 0;
    for (const s of states) {
      const n = Number(s.n);
      candidates += n;
      const reason = SUPPRESSION_BY_STATE[s.state];
      if (reason) suppressed[reason] = (suppressed[reason] ?? 0) + n;
    }
    const run = new Set<string>();
    const notRun = new Map<string, string>();
    for (const r of reviewers) {
      if (r.state === 'succeeded') run.add(r.reviewer);
      else if (r.state !== 'pending' && r.state !== 'running') {
        notRun.set(r.reviewer, r.error_class ? `${r.state} (${r.error_class})` : r.state);
      }
    }
    for (const r of ctx.degradedReviewers) if (!notRun.has(r)) notRun.set(r, 'degraded');
    const reviewersNotRun = [...notRun]
      .filter(([r]) => !run.has(r) || ctx.degradedReviewers.includes(r))
      .map(([reviewer, reason]) => ({ reviewer, reason }));
    return {
      runId: ctx.runId,
      headSha: ctx.headSha,
      degraded: reviewersNotRun.length > 0,
      changed: { files: changedFiles, behavioralSymbols: 0, apiContracts: 0 },
      verified: Number(verified?.n ?? 0),
      candidates,
      suppressed,
      coverage: {
        reviewersRun: [...run].sort(),
        reviewersNotRun,
        unreviewedClusters: [],
        checks: [],
      },
    };
  }
}
