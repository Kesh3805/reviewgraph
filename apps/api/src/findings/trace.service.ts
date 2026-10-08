import { Injectable, NotFoundException } from '@nestjs/common';
import { sql } from 'kysely';
import { DbService } from '../db/db.module';
import type { EvidenceItem, FindingTrace } from './dto/finding.dto';
import {
  candidateSeverity,
  evidenceItems,
  loadFinding,
  publicationOf,
  severityOf,
  suppressionOf,
} from './finding-rows';

const OUTCOMES = new Set(['pass', 'fail', 'inconclusive']);
/** The verification pipeline has eight stages (ADR-011). */
const VERIFICATION_STAGES = 8;

/**
 * The explainability trace of a finding (API-010, PRD sections 85 and 86): from the changed
 * lines through the symbols, context references and reviewer to the candidate, every
 * verification stage, dedup merges, the effective policy and the publication. Prompts and raw
 * model output are never read, so they cannot leak into the trace.
 */
@Injectable()
export class FindingTraceService {
  constructor(private readonly dbs: DbService) {}

  async trace(orgId: string, findingId: string): Promise<FindingTrace> {
    return this.dbs.withTx(orgId, async (trx) => {
      const row = await loadFinding(trx, findingId);
      if (!row) throw new NotFoundException();
      const run = await trx
        .selectFrom('review_runs')
        .select(['pull_request_id', 'base_sha', 'head_sha'])
        .where('id', '=', row.review_run_id)
        .executeTakeFirstOrThrow();
      const reviewerRun = await trx
        .selectFrom('reviewer_runs')
        .select(['reviewer', 'reviewer_version', 'model'])
        .where('id', '=', row.reviewer_run_id)
        .executeTakeFirstOrThrow();
      const mergedFrom = await trx
        .selectFrom('candidate_findings')
        .select('id')
        .where('review_run_id', '=', row.review_run_id)
        .where('state', '=', 'SUPPRESSED_DUPLICATE')
        .where(sql<boolean>`suppression -> 'reason' ->> 'of' = ${row.c_id}`)
        .orderBy('id')
        .execute();

      const candidateEvidence = evidenceItems(row.c_evidence);
      const verifiedEvidence = evidenceItems(row.v_evidence);
      const stages = stagesOf(row.stage_outcomes);
      const suppression = suppressionOf(row);
      // Suppressed before verification finished: the later stages legitimately did not run.
      const stoppedEarly = suppression !== null;
      const incomplete =
        !reviewerRun.reviewer_version ||
        (row.v_id !== null && stages.length < VERIFICATION_STAGES && !stoppedEarly) ||
        (row.v_id === null && !stoppedEarly);

      return {
        finding_id: row.v_id ?? row.c_id,
        change: {
          review_run_id: row.review_run_id,
          pull_request_id: run.pull_request_id,
          base_sha: run.base_sha,
          head_sha: run.head_sha,
          anchor: {
            path: row.changed_path,
            side: row.changed_side === 'base' ? 'base' : 'head',
            start_line: row.changed_start_line,
            end_line: row.changed_end_line,
          },
        },
        symbols: row.affected_symbols,
        context_refs: contextRefs([...candidateEvidence, ...verifiedEvidence]),
        symbol_path: symbolPath([...verifiedEvidence, ...candidateEvidence]),
        reviewer: {
          type: reviewerRun.reviewer,
          version: reviewerRun.reviewer_version,
          model: reviewerRun.model,
          reviewer_run_id: row.reviewer_run_id,
        },
        candidate: {
          id: row.c_id,
          state: row.c_state,
          severity: candidateSeverity(row),
          fingerprint: row.fingerprint,
          created_at: row.c_created_at.toISOString(),
        },
        verification:
          row.v_id !== null && row.computed_confidence !== null && row.verification_version !== null
            ? {
                verified_id: row.v_id,
                version: row.verification_version,
                stages,
                evidence: verifiedEvidence,
                confidence: row.computed_confidence,
                severity: severityOf(row),
              }
            : null,
        dedup: {
          merged_into: suppression?.duplicate_of ?? null,
          merged_from: mergedFrom.map((m) => m.id),
        },
        policy: { band: row.band, suppression },
        publication: publicationOf(row),
        incomplete,
      };
    });
  }
}

type StageRecord = NonNullable<FindingTrace['verification']>['stages'][number];

function stagesOf(value: unknown): StageRecord[] {
  if (!Array.isArray(value)) return [];
  return value
    .flatMap((s: unknown) => {
      if (!s || typeof s !== 'object') return [];
      const { stage, outcome, reason } = s as Record<string, unknown>;
      if (typeof stage !== 'number' || typeof outcome !== 'string' || !OUTCOMES.has(outcome)) {
        return [];
      }
      return [
        {
          stage,
          outcome: outcome as 'pass' | 'fail' | 'inconclusive',
          reason: typeof reason === 'string' ? reason : null,
        },
      ];
    })
    .sort((a, b) => a.stage - b.stage);
}

interface Location {
  path?: unknown;
  lines?: { start?: unknown; end?: unknown };
  snapshot_id?: unknown;
}

function contextRefs(items: EvidenceItem[]): FindingTrace['context_refs'] {
  const seen = new Set<string>();
  const refs: FindingTrace['context_refs'] = [];
  for (const item of items) {
    const loc = item.location as Location | null;
    if (!loc || typeof loc.path !== 'string') continue;
    const start = typeof loc.lines?.start === 'number' ? loc.lines.start : null;
    const end = typeof loc.lines?.end === 'number' ? loc.lines.end : null;
    const snapshot = typeof loc.snapshot_id === 'string' ? loc.snapshot_id : null;
    const key = `${loc.path}:${start}:${end}:${snapshot}`;
    if (seen.has(key)) continue;
    seen.add(key);
    refs.push({ path: loc.path, start_line: start, end_line: end, snapshot_id: snapshot });
  }
  return refs;
}

interface SymbolRefLike {
  key?: unknown;
  display?: unknown;
}

/** `from -> via... -> to` of the first evidence item that claims a relation path. */
function symbolPath(items: EvidenceItem[]): string[] {
  for (const item of items) {
    const rel = item.relation as { from?: SymbolRefLike; to?: SymbolRefLike; via?: unknown } | null;
    if (!rel?.from || !rel.to) continue;
    const via = Array.isArray(rel.via) ? (rel.via as SymbolRefLike[]) : [];
    const names = [rel.from, ...via, rel.to].map((s) =>
      typeof s.display === 'string' ? s.display : typeof s.key === 'string' ? s.key : '?',
    );
    return names;
  }
  return [];
}
