/**
 * TanStack Query keys and options for every screen. Fetching lives in `api/endpoints.ts`;
 * this module adds cache keys and polling rules.
 */
import { queryOptions } from '@tanstack/react-query';
import * as endpoints from './api/endpoints';
import type {
  Page,
  PullRequestListQuery,
  PullRequestSummary,
  RepositoryStatus,
  UsageGroupBy,
} from './api/pending';
import { isTerminalReview } from './run-state';

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
  review: (reviewId: string) => ['review', reviewId] as const,
  reviewFindings: (reviewId: string) => ['review', reviewId, 'findings'] as const,
  reviewHistory: (prId: string) => ['pull-request', prId, 'reviews'] as const,
  finding: (findingId: string) => ['finding', findingId] as const,
  findingTrace: (findingId: string) => ['finding', findingId, 'trace'] as const,
  source: (repoId: string, req: object) => ['source', repoId, req] as const,
  feedback: (findingId: string) => ['finding', findingId, 'feedback'] as const,
  profile: (repoId: string) => ['repository', repoId, 'profile'] as const,
  intelligence: (repoId: string) => ['repository', repoId, 'intelligence'] as const,
  rules: (repoId: string) => ['repository', repoId, 'rules'] as const,
  suppressions: (repoId: string) => ['repository', repoId, 'suppressions'] as const,
  integration: (orgId: string) => ['organization', orgId, 'integrations', 'github'] as const,
  usage: (orgId: string, query: object) => ['organization', orgId, 'usage', query] as const,
  orgSettings: (orgId: string) => ['organization', orgId, 'settings'] as const,
  members: (orgId: string) => ['organization', orgId, 'members'] as const,
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

/** Rows with a non-terminal latest run refetch every 5 s (WEB-005). */
export const RUN_POLL_MS = 5000;

export function pullsRefetchInterval(page: Page<PullRequestSummary> | undefined): number | false {
  return page?.items.some((pr) => pr.latest_run && !isTerminalReview(pr.latest_run.state))
    ? RUN_POLL_MS
    : false;
}

export function pullRequestsQuery(query: PullRequestListQuery) {
  return queryOptions({
    queryKey: keys.pullRequests(query),
    queryFn: ({ signal }) => endpoints.listPullRequests(query, { signal }),
    refetchInterval: (q) => pullsRefetchInterval(q.state.data),
    throwOnError: false,
  });
}

/** Review detail polls while the run is non-terminal (WEB-006). */
export function reviewQuery(reviewId: string) {
  return queryOptions({
    queryKey: keys.review(reviewId),
    queryFn: ({ signal }) => endpoints.getReview(reviewId, { signal }),
    refetchInterval: (q) =>
      q.state.data && !isTerminalReview(q.state.data.state) ? RUN_POLL_MS : false,
  });
}

export function reviewFindingsQuery(reviewId: string, live = false) {
  return queryOptions({
    queryKey: keys.reviewFindings(reviewId),
    queryFn: ({ signal }) => endpoints.listReviewFindings(reviewId, 'all', { signal }),
    refetchInterval: live ? RUN_POLL_MS : false,
    throwOnError: false,
  });
}

export function reviewHistoryQuery(prId: string) {
  return queryOptions({
    queryKey: keys.reviewHistory(prId),
    queryFn: ({ signal }) => endpoints.listReviewHistory(prId, { signal }),
    throwOnError: false,
  });
}

export function findingQuery(findingId: string) {
  return queryOptions({
    queryKey: keys.finding(findingId),
    queryFn: ({ signal }) => endpoints.getFinding(findingId, { signal }),
  });
}

export function findingTraceQuery(findingId: string) {
  return queryOptions({
    queryKey: keys.findingTrace(findingId),
    queryFn: ({ signal }) => endpoints.getFindingTrace(findingId, { signal }),
    throwOnError: false,
  });
}

export function feedbackQuery(findingId: string) {
  return queryOptions({
    queryKey: keys.feedback(findingId),
    queryFn: ({ signal }) => endpoints.getFindingFeedback(findingId, { signal }),
    throwOnError: false,
  });
}

export interface ExcerptRequest {
  path: string;
  start: number;
  end: number;
  snapshot?: string;
}

export function sourceExcerptQuery(repoId: string, req: ExcerptRequest) {
  return queryOptions({
    queryKey: keys.source(repoId, req),
    queryFn: ({ signal }) => endpoints.getSourceExcerpt(repoId, req, { signal }),
    // Snapshots are immutable: an excerpt never changes.
    staleTime: Infinity,
    throwOnError: false,
  });
}

export function repositoryProfileQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.profile(repoId),
    queryFn: ({ signal }) => endpoints.getRepositoryProfile(repoId, { signal }),
    throwOnError: false,
  });
}

export function repositoryIntelligenceQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.intelligence(repoId),
    queryFn: ({ signal }) => endpoints.getRepositoryIntelligence(repoId, { signal }),
    throwOnError: false,
  });
}

export function repositoryRulesQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.rules(repoId),
    queryFn: ({ signal }) => endpoints.getRepositoryRules(repoId, { signal }),
    throwOnError: false,
  });
}

export function suppressionsQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.suppressions(repoId),
    queryFn: ({ signal }) => endpoints.listSuppressions(repoId, { signal }),
    throwOnError: false,
  });
}

export function githubIntegrationQuery(orgId: string) {
  return queryOptions({
    queryKey: keys.integration(orgId),
    queryFn: ({ signal }) => endpoints.getGithubIntegration(orgId, { signal }),
    throwOnError: false,
  });
}

export function usageQuery(
  orgId: string,
  query: { from: string; to: string; group_by: UsageGroupBy },
) {
  return queryOptions({
    queryKey: keys.usage(orgId, query),
    queryFn: ({ signal }) => endpoints.getUsage(orgId, query, { signal }),
    throwOnError: false,
  });
}

export function orgSettingsQuery(orgId: string) {
  return queryOptions({
    queryKey: keys.orgSettings(orgId),
    queryFn: ({ signal }) => endpoints.getOrganizationSettings(orgId, { signal }),
    throwOnError: false,
  });
}

export function membersQuery(orgId: string) {
  return queryOptions({
    queryKey: keys.members(orgId),
    queryFn: ({ signal }) => endpoints.listMembers(orgId, { signal }),
    throwOnError: false,
  });
}

export function riskAreasQuery(repoId: string) {
  return queryOptions({
    queryKey: keys.riskAreas(repoId),
    queryFn: ({ signal }) => endpoints.listRiskAreas(repoId, { signal }),
    throwOnError: false,
  });
}
