import type { FindingFeedback, Verdict } from './api/pending';

export const VERDICTS: { value: Verdict; label: string }[] = [
  { value: 'useful', label: 'Useful' },
  { value: 'false_positive', label: 'False positive' },
  { value: 'already_handled', label: 'Already handled' },
  { value: 'not_relevant', label: 'Not relevant' },
  { value: 'intentional', label: 'Intentional' },
];

/** Verdicts that may also create a suppression (API-012, POL-006). */
export const SUPPRESSIBLE: ReadonlySet<Verdict> = new Set(['intentional', 'not_relevant']);

export const MAX_COMMENT_LENGTH = 2000;

export function emptyCounts(): Record<Verdict, number> {
  return {
    useful: 0,
    false_positive: 0,
    already_handled: 0,
    not_relevant: 0,
    intentional: 0,
  };
}

/** The optimistic state after the caller sets `verdict` (latest wins: the old one is undone). */
export function applyVerdict(
  prev: FindingFeedback | undefined,
  verdict: Verdict,
  comment: string | null,
  now = new Date().toISOString(),
): FindingFeedback {
  const counts = { ...emptyCounts(), ...prev?.counts };
  const old = prev?.mine?.verdict;
  if (old) counts[old] = Math.max(0, counts[old] - 1);
  counts[verdict] += 1;
  return { mine: { verdict, comment, updated_at: now }, counts };
}
