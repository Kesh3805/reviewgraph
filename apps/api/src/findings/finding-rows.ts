import { sql } from 'kysely';
import type { Tx } from '../db/tx';
import type { EvidenceItem, FindingSummary, Severity } from './dto/finding.dto';
import { SEVERITIES } from './dto/finding.dto';

/**
 * A finding as stored: the candidate, its verified row (if verification ran) and its publication
 * (if published). Columns that hold model reasoning (`reasoning_artifacts`) are never selected.
 */
export interface FindingRow {
  c_id: string;
  review_run_id: string;
  reviewer_run_id: string;
  reviewer: string;
  category: string;
  title: string;
  description: string;
  changed_path: string;
  changed_side: string;
  changed_start_line: number;
  changed_end_line: number;
  severity_candidate: string;
  affected_symbols: string[];
  c_evidence: unknown;
  fingerprint: string;
  c_state: string;
  suppression: unknown;
  suppressed_at_stage: number | null;
  c_created_at: Date;
  v_id: string | null;
  computed_confidence: number | null;
  v_severity: string | null;
  band: string | null;
  verification_version: number | null;
  stage_outcomes: unknown;
  v_evidence: unknown;
  priority_score: number | null;
  p_id: string | null;
  placement: string | null;
  p_path: string | null;
  p_start_line: number | null;
  p_end_line: number | null;
  p_head_sha: string | null;
  provider_review_id: string | null;
  provider_comment_id: string | null;
  published_at: Date | null;
}

export function selectFindings(trx: Tx) {
  return trx
    .selectFrom('candidate_findings as c')
    .leftJoin('verified_findings as v', 'v.candidate_finding_id', 'c.id')
    .leftJoin('published_findings as p', 'p.verified_finding_id', 'v.id')
    .select([
      'c.id as c_id',
      'c.review_run_id',
      'c.reviewer_run_id',
      'c.reviewer',
      'c.category',
      'c.title',
      'c.description',
      'c.changed_path',
      'c.changed_side',
      'c.changed_start_line',
      'c.changed_end_line',
      'c.severity_candidate',
      'c.affected_symbols',
      'c.evidence as c_evidence',
      'c.fingerprint',
      'c.state as c_state',
      'c.suppression',
      'c.suppressed_at_stage',
      'c.created_at as c_created_at',
      'v.id as v_id',
      'v.computed_confidence',
      'v.severity as v_severity',
      'v.band',
      'v.verification_version',
      'v.stage_outcomes',
      'v.evidence as v_evidence',
      'v.priority_score',
      'p.id as p_id',
      'p.placement',
      'p.path as p_path',
      'p.start_line as p_start_line',
      'p.end_line as p_end_line',
      'p.head_sha as p_head_sha',
      'p.provider_review_id',
      'p.provider_comment_id',
      'p.published_at',
    ]);
}

/** A finding by its verified id or, for one that was never verified, its candidate id. */
export async function loadFinding(trx: Tx, findingId: string): Promise<FindingRow | undefined> {
  const row = await selectFindings(trx)
    .where((eb) => eb.or([eb('v.id', '=', findingId), eb('c.id', '=', findingId)]))
    .executeTakeFirst();
  return row as FindingRow | undefined;
}

export const isSuppressed = (state: string): boolean =>
  state.startsWith('SUPPRESSED_') || state === 'INVALIDATED';

export function lifecycleOf(row: FindingRow): FindingSummary['lifecycle'] {
  if (row.p_id) return 'published';
  if (isSuppressed(row.c_state)) return 'suppressed';
  if (row.v_id) return 'verified';
  return 'candidate';
}

const asSeverity = (value: string | null): Severity =>
  (SEVERITIES as readonly string[]).includes(value ?? '') ? (value as Severity) : 'info';

export function severityOf(row: FindingRow): Severity {
  return asSeverity(row.v_severity ?? row.severity_candidate);
}

export function candidateSeverity(row: FindingRow): Severity {
  return asSeverity(row.severity_candidate);
}

export function suppressionOf(row: FindingRow): FindingSummary['suppression'] {
  if (!row.suppression || typeof row.suppression !== 'object') return null;
  const s = row.suppression as {
    reason?: { type?: unknown; of?: unknown };
    detail?: unknown;
    stage?: unknown;
  };
  const reason = typeof s.reason?.type === 'string' ? s.reason.type : row.c_state.toLowerCase();
  return {
    reason,
    detail: typeof s.detail === 'string' ? s.detail : null,
    stage: typeof s.stage === 'number' ? s.stage : (row.suppressed_at_stage ?? null),
    duplicate_of: reason === 'duplicate' && typeof s.reason?.of === 'string' ? s.reason.of : null,
  };
}

export function toSummary(row: FindingRow): FindingSummary {
  return {
    id: row.v_id ?? row.c_id,
    candidate_id: row.c_id,
    verified_id: row.v_id,
    review_run_id: row.review_run_id,
    lifecycle: lifecycleOf(row),
    state: row.c_state,
    reviewer: row.reviewer,
    category: row.category,
    title: row.title,
    severity: severityOf(row),
    confidence: row.computed_confidence,
    band: row.band,
    anchor: {
      path: row.changed_path,
      side: row.changed_side === 'base' ? 'base' : 'head',
      start_line: row.changed_start_line,
      end_line: row.changed_end_line,
    },
    suppression: suppressionOf(row),
    published:
      row.p_id && row.placement && row.published_at
        ? {
            placement: row.placement,
            published_at: row.published_at.toISOString(),
            provider_comment_id: row.provider_comment_id,
          }
        : null,
    created_at: row.c_created_at.toISOString(),
  };
}

export function publicationOf(row: FindingRow) {
  if (!row.p_id || !row.placement || !row.published_at || !row.p_head_sha) return null;
  return {
    id: row.p_id,
    placement: row.placement === 'inline' ? ('inline' as const) : ('summary' as const),
    path: row.p_path,
    start_line: row.p_start_line,
    end_line: row.p_end_line,
    head_sha: row.p_head_sha,
    provider_review_id: row.provider_review_id,
    provider_comment_id: row.provider_comment_id,
    published_at: row.published_at.toISOString(),
  };
}

const str = (v: unknown): string | null => (typeof v === 'string' ? v : null);

/**
 * Evidence items reduced to the typed DOM-007 fields. Anything else a row might carry (raw model
 * output, prompts, excerpts) is dropped here, whatever wrote the row.
 */
export function evidenceItems(value: unknown): EvidenceItem[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item: unknown) => {
    if (!item || typeof item !== 'object') return [];
    const e = item as Record<string, unknown>;
    if (typeof e.kind !== 'string') return [];
    return [
      {
        kind: e.kind,
        claimed_strength: str(e.claimed_strength),
        origin: e.origin ?? null,
        verification: e.verification ?? null,
        claim: str(e.claim),
        location: e.location ?? null,
        symbols: Array.isArray(e.symbols) ? e.symbols : [],
        relation: e.relation ?? null,
      },
    ];
  });
}

/** Orders by severity (critical first), then priority, then age. */
export const severityOrder = sql<number>`case coalesce(v.severity, c.severity_candidate)
  when 'critical' then 4 when 'high' then 3 when 'medium' then 2 when 'low' then 1 else 0 end`;
