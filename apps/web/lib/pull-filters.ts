import type { PullRequestListQuery, Severity } from './api/pending';
import { SEVERITY_ORDER } from './severity';

/** Filters of the pull request list. They live in the URL so views can be shared. */
export interface PullFilters {
  repository?: string;
  state?: 'open' | 'closed';
  hasFindings?: boolean;
  severity?: Severity;
  cursor?: string;
}

type Params = { get(name: string): string | null };

export function parsePullFilters(params: Params): PullFilters {
  const filters: PullFilters = {};
  const repository = params.get('repository');
  if (repository) filters.repository = repository;
  const state = params.get('state');
  if (state === 'open' || state === 'closed') filters.state = state;
  const has = params.get('has_findings');
  if (has === 'true' || has === 'false') filters.hasFindings = has === 'true';
  const severity = params.get('severity');
  if (severity && (SEVERITY_ORDER as string[]).includes(severity)) {
    filters.severity = severity as Severity;
  }
  const cursor = params.get('cursor');
  if (cursor) filters.cursor = cursor;
  return filters;
}

export function pullFiltersToSearch(filters: PullFilters): string {
  const params = new URLSearchParams();
  if (filters.repository) params.set('repository', filters.repository);
  if (filters.state) params.set('state', filters.state);
  if (filters.hasFindings !== undefined) params.set('has_findings', String(filters.hasFindings));
  if (filters.severity) params.set('severity', filters.severity);
  if (filters.cursor) params.set('cursor', filters.cursor);
  const s = params.toString();
  return s ? `?${s}` : '';
}

/** The API query for a filter set; a fixed repository (repository tab) wins over the filter. */
export function toListQuery(
  filters: PullFilters,
  scope: { orgId: string; repoId?: string },
  limit = 25,
): PullRequestListQuery {
  const repositoryId = scope.repoId ?? filters.repository;
  return {
    ...(repositoryId ? { repository_id: repositoryId } : { organization_id: scope.orgId }),
    state: filters.state,
    has_findings: filters.hasFindings,
    severity: filters.severity,
    cursor: filters.cursor,
    limit,
  };
}
