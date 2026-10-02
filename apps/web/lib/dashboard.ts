import { queryOptions } from '@tanstack/react-query';
import { api } from './api-client';

/** Run states from the review state machine; the last three are terminal. */
export type RunState =
  | 'QUEUED'
  | 'INDEXING'
  | 'ANALYZING'
  | 'REVIEWING'
  | 'VERIFYING'
  | 'PUBLISHING'
  | 'COMPLETED'
  | 'DEGRADED'
  | 'FAILED'
  | 'SUPERSEDED'
  | 'CANCELLED';

const TERMINAL_STATES: ReadonlySet<string> = new Set([
  'COMPLETED',
  'DEGRADED',
  'FAILED',
  'SUPERSEDED',
  'CANCELLED',
]);

export type Severity = 'critical' | 'high' | 'medium' | 'low' | 'info';

export interface ActiveRun {
  id: string;
  repository: string;
  pull_request_number: number;
  state: RunState;
  stage: string | null;
  started_at: string;
}

/**
 * Shape of `GET /organizations/:id/dashboard` (7-day window). Sections may be null when the
 * API could not compute them; the UI then shows a card-level "unavailable" state.
 */
export interface DashboardSummary {
  window_days: number;
  reviews: { completed: number; degraded: number; failed: number } | null;
  latency_ms: { median: number; p95: number } | null;
  findings_by_severity: Record<Severity, number> | null;
  /** Rates are fractions in [0, 1]. */
  quality: { acceptance_rate: number; false_positive_rate: number } | null;
  active_runs: ActiveRun[];
}

export const ACTIVE_RUNS_POLL_MS = 5000;

export function isTerminal(state: string): boolean {
  return TERMINAL_STATES.has(state);
}

/** Poll while any run is non-terminal; stop once everything has settled. */
export function activeRunsRefetchInterval(data: DashboardSummary | undefined): number | false {
  return data?.active_runs.some((run) => !isTerminal(run.state)) ? ACTIVE_RUNS_POLL_MS : false;
}

export function dashboardQuery(orgId: string) {
  return queryOptions({
    queryKey: ['dashboard', orgId],
    queryFn: async ({ signal }) => {
      const { data } = await api.GET('/api/v1/organizations/{id}/dashboard', {
        params: { path: { id: orgId } },
        signal,
      });
      if (!data) throw new Error('empty dashboard response');
      return data;
    },
    refetchInterval: (query) => activeRunsRefetchInterval(query.state.data),
    // Failures are rendered per card instead of tripping the page error boundary.
    throwOnError: false,
  });
}
