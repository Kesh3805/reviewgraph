/**
 * TanStack Query keys and options for every screen. Fetching lives in `api/endpoints.ts`;
 * this module adds cache keys and polling rules.
 */
import { queryOptions } from '@tanstack/react-query';
import * as endpoints from './api/endpoints';
import type { PullRequestListQuery, RepositoryStatus } from './api/pending';

/** Status card / active job polling interval (WEB-003). */
export const INDEX_POLL_MS = 5000;

export function isIndexActive(status: RepositoryStatus | undefined): boolean {
  return (
    !!status &&
    (status.active_job !== null ||
      status.index_state === 'queued' ||
      status.index_state === 'indexing')
  );
}

export const keys = {
  repositories: (orgId: string) => ['repositories', orgId] as const,
  repository: (repoId: string) => ['repository', repoId] as const,
  repositoryStatus: (repoId: string) => ['repository', repoId, 'status'] as const,
  installationRepositories: (orgId: string) => ['installation-repositories', orgId] as const,
  repositoryActivity: (orgId: string) => ['repository-activity', orgId] as const,
  riskAreas: (repoId: string) => ['repository', repoId, 'risk-areas'] as const,
  pullRequests: (query: PullRequestListQuery) => ['pull-requests', query] as const,
};

export function repositoriesQuery(orgId: string) {
  return queryOptions({
    queryKey: keys.repositories(orgId),
    queryFn: ({ signal }) => endpoints.listRepositories(orgId, { signal, limit: 100 }),
  });
}

export function repositoryQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.repository(repoId),
    queryFn: ({ signal }) => endpoints.getRepository(repoId, { signal }),
  });
}

export function repositoryStatusQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.repositoryStatus(repoId),
    queryFn: ({ signal }) => endpoints.getRepositoryStatus(repoId, { signal }),
    // Poll only while an index job is queued or running.
    refetchInterval: (query) => (isIndexActive(query.state.data) ? INDEX_POLL_MS : false),
    throwOnError: false,
  });
}

export function installationRepositoriesQuery(orgId: string) {
  return queryOptions({
    queryKey: keys.installationRepositories(orgId),
    queryFn: ({ signal }) => endpoints.listInstallationRepositories(orgId, { signal }),
    throwOnError: false,
  });
}

export function repositoryActivityQuery(orgId: string) {
  return queryOptions({
    queryKey: keys.repositoryActivity(orgId),
    queryFn: ({ signal }) => endpoints.listRepositoryActivity(orgId, { signal }),
    // Optional enrichment: a failure leaves dashes in the table.
    throwOnError: false,
    retry: false,
  });
}

export function riskAreasQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.riskAreas(repoId),
    queryFn: ({ signal }) => endpoints.listRiskAreas(repoId, { signal }),
    throwOnError: false,
  });
}
