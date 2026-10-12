import type { BadgeVariant } from '@/components/ui/badge';
import type { ReviewState } from './api/pending';

export const REVIEW_STATES: readonly ReviewState[] = [
  'RECEIVED',
  'INDEXING',
  'ANALYZING',
  'REVIEWING',
  'VERIFYING',
  'PUBLISHING',
  'COMPLETED',
  'FAILED_INDEXING',
  'FAILED_ANALYSIS',
  'FAILED_REVIEW',
  'FAILED_PUBLISH',
  'SUPERSEDED',
  'CANCELLED',
];

const TERMINAL: ReadonlySet<ReviewState> = new Set([
  'COMPLETED',
  'FAILED_INDEXING',
  'FAILED_ANALYSIS',
  'FAILED_REVIEW',
  'FAILED_PUBLISH',
  'SUPERSEDED',
  'CANCELLED',
]);

export function isTerminalReview(state: ReviewState): boolean {
  return TERMINAL.has(state);
}

/** Label and colour for a run state; a degraded COMPLETED run is never shown as plain green. */
export function runStateDisplay(
  state: ReviewState,
  degraded = false,
): { label: string; variant: BadgeVariant } {
  switch (state) {
    case 'RECEIVED':
      return { label: 'Queued', variant: 'info' };
    case 'INDEXING':
      return { label: 'Indexing', variant: 'info' };
    case 'ANALYZING':
      return { label: 'Analyzing', variant: 'info' };
    case 'REVIEWING':
      return { label: 'Reviewing', variant: 'info' };
    case 'VERIFYING':
      return { label: 'Verifying', variant: 'info' };
    case 'PUBLISHING':
      return { label: 'Publishing', variant: 'info' };
    case 'COMPLETED':
      return degraded
        ? { label: 'Completed (degraded)', variant: 'warning' }
        : { label: 'Completed', variant: 'success' };
    case 'FAILED_INDEXING':
      return { label: 'Failed: indexing', variant: 'danger' };
    case 'FAILED_ANALYSIS':
      return { label: 'Failed: analysis', variant: 'danger' };
    case 'FAILED_REVIEW':
      return { label: 'Failed: review', variant: 'danger' };
    case 'FAILED_PUBLISH':
      return { label: 'Failed: publish', variant: 'danger' };
    case 'SUPERSEDED':
      return { label: 'Superseded', variant: 'muted' };
    case 'CANCELLED':
      return { label: 'Cancelled', variant: 'muted' };
  }
}
