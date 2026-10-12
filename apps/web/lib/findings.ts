import type { FindingState, FindingSummary } from './api/pending';
import { SEVERITY_ORDER } from './severity';

export const SUPPRESSION_LABELS: Partial<Record<FindingState, string>> = {
  SUPPRESSED_LOW_CONFIDENCE: 'Low confidence',
  SUPPRESSED_DUPLICATE: 'Duplicate',
  SUPPRESSED_PREEXISTING: 'Pre-existing',
  SUPPRESSED_NOT_ACTIONABLE: 'Not actionable',
  SUPPRESSED_POLICY: 'Policy',
  INVALIDATED: 'Invalidated by verification',
};

export function isSuppressed(state: FindingState): boolean {
  return state in SUPPRESSION_LABELS;
}

/** Severity first (critical..info), then confidence descending, then path. */
export function sortFindings(items: FindingSummary[]): FindingSummary[] {
  return [...items].sort(
    (a, b) =>
      SEVERITY_ORDER.indexOf(a.severity) - SEVERITY_ORDER.indexOf(b.severity) ||
      b.confidence - a.confidence ||
      a.anchor.path.localeCompare(b.anchor.path) ||
      a.anchor.start_line - b.anchor.start_line,
  );
}

export interface FindingGroups {
  /** Published inline on the diff. */
  published: FindingSummary[];
  /** Published but anchored outside the diff (summary only). */
  relocated: FindingSummary[];
  /** Suppressed, grouped by reason in a stable order. */
  suppressed: { state: FindingState; label: string; items: FindingSummary[] }[];
  /** Not yet through the pipeline (the run is still going). */
  pending: FindingSummary[];
}

export function groupFindings(items: FindingSummary[]): FindingGroups {
  const sorted = sortFindings(items);
  const groups: FindingGroups = { published: [], relocated: [], suppressed: [], pending: [] };
  const byReason = new Map<FindingState, FindingSummary[]>();
  for (const f of sorted) {
    if (f.state === 'PUBLISHED') (f.relocated ? groups.relocated : groups.published).push(f);
    else if (isSuppressed(f.state)) byReason.set(f.state, [...(byReason.get(f.state) ?? []), f]);
    else groups.pending.push(f);
  }
  for (const state of Object.keys(SUPPRESSION_LABELS) as FindingState[]) {
    const list = byReason.get(state);
    if (list)
      groups.suppressed.push({ state, label: SUPPRESSION_LABELS[state] ?? state, items: list });
  }
  return groups;
}

/** Findings per changed file, for the Files section. */
export function findingsByPath(items: FindingSummary[]): Map<string, FindingSummary[]> {
  const map = new Map<string, FindingSummary[]>();
  for (const f of items) map.set(f.anchor.path, [...(map.get(f.anchor.path) ?? []), f]);
  return map;
}
