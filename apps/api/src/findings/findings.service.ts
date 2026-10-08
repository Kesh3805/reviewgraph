import { Injectable, NotFoundException } from '@nestjs/common';
import { sql } from 'kysely';
import { DbService } from '../db/db.module';
import type { FindingDetail, FindingSummary, ListFindingsQuery } from './dto/finding.dto';
import {
  candidateSeverity,
  evidenceItems,
  loadFinding,
  publicationOf,
  selectFindings,
  severityOrder,
  toSummary,
  type FindingRow,
} from './finding-rows';

/** Findings by lifecycle state and finding detail (API-010). Read-only and tenant scoped. */
@Injectable()
export class FindingsService {
  constructor(private readonly dbs: DbService) {}

  async listForReview(
    orgId: string,
    reviewId: string,
    query: ListFindingsQuery,
  ): Promise<{ items: FindingSummary[] }> {
    const rows = await this.dbs.withTx(orgId, async (trx) => {
      const run = await trx
        .selectFrom('review_runs')
        .select('id')
        .where('id', '=', reviewId)
        .executeTakeFirst();
      if (!run) throw new NotFoundException();
      let q = selectFindings(trx).where('c.review_run_id', '=', reviewId);
      if (query.state === 'published') q = q.where('p.id', 'is not', null);
      if (query.state === 'verified') q = q.where('v.id', 'is not', null);
      if (query.state === 'suppressed') {
        q = q.where((eb) =>
          eb.or([eb('c.state', 'like', 'SUPPRESSED\\_%'), eb('c.state', '=', 'INVALIDATED')]),
        );
      }
      if (query.severity?.length) {
        q = q.where(
          sql<boolean>`coalesce(v.severity, c.severity_candidate) = any(${query.severity}::text[])`,
        );
      }
      if (query.reviewer?.length) q = q.where('c.reviewer', 'in', query.reviewer);
      return q
        .orderBy(severityOrder, 'desc')
        .orderBy(sql`v.priority_score desc nulls last`)
        .orderBy('c.created_at')
        .orderBy('c.id')
        .execute();
    });
    return { items: (rows as FindingRow[]).map(toSummary) };
  }

  async detail(orgId: string, findingId: string): Promise<FindingDetail> {
    return this.dbs.withTx(orgId, async (trx) => {
      const row = await loadFinding(trx, findingId);
      if (!row) throw new NotFoundException();
      const run = await trx
        .selectFrom('review_runs')
        .select(['repository_id', 'pull_request_id'])
        .where('id', '=', row.review_run_id)
        .executeTakeFirstOrThrow();
      return toDetail(row, run);
    });
  }
}

function confidenceComponents(stageOutcomes: unknown): Record<string, number> | null {
  // VER-009 records the components next to the stage outcomes when it computes them.
  if (!stageOutcomes || typeof stageOutcomes !== 'object' || Array.isArray(stageOutcomes)) {
    return null;
  }
  const components = (stageOutcomes as { confidence_components?: unknown }).confidence_components;
  if (!components || typeof components !== 'object') return null;
  const entries = Object.entries(components).filter(
    (e): e is [string, number] => typeof e[1] === 'number',
  );
  return Object.fromEntries(entries);
}

export function toDetail(
  row: FindingRow,
  run: { repository_id: string; pull_request_id: string },
): FindingDetail {
  return {
    ...toSummary(row),
    explanation: row.description,
    symbols: row.affected_symbols,
    evidence: evidenceItems(row.v_evidence ?? row.c_evidence),
    confidence_detail:
      row.computed_confidence === null
        ? null
        : {
            value: row.computed_confidence,
            components: confidenceComponents(row.stage_outcomes),
          },
    severity_candidate: candidateSeverity(row),
    verification_version: row.verification_version,
    publication: publicationOf(row),
    repository_id: run.repository_id,
    pull_request_id: run.pull_request_id,
  };
}
