/**
 * Every API call the screens make, in one typed module. Routes come from `schema.ts`, so a
 * call to a route that is not generated yet is typed by `pending.ts`.
 *
 * Errors surface as `ApiError` (see `api-client.ts`); these helpers only unwrap `data`.
 */
import { api } from '../api-client';
import type {
  FindingDetail,
  FindingTrace,
  SourceExcerpt,
  FindingStateFilter,
  FindingSummary,
  ReviewDetail,
  ReviewRunSummary,
  IndexRequest,
  InstallationRepository,
  ManualReviewResponse,
  Page,
  PullRequestListQuery,
  PullRequestSummary,
  Repository,
  RiskArea,
  RepositoryActivity,
  RepositoryList,
  RepositorySettings,
  RepositorySettingsPatch,
  RepositoryStatus,
} from './pending';

type Signal = { signal?: AbortSignal };

function required<T>(data: T | undefined, what: string): T {
  if (data === undefined) throw new Error(`empty ${what} response`);
  return data;
}

// ---------------------------------------------------------------------------------------------
// Repositories (API-008)
// ---------------------------------------------------------------------------------------------

export async function listRepositories(
  organizationId: string,
  opts: Signal & { cursor?: string; limit?: number } = {},
): Promise<RepositoryList> {
  const { data } = await api.GET('/api/v1/repositories', {
    params: { query: { organization_id: organizationId, cursor: opts.cursor, limit: opts.limit } },
    signal: opts.signal,
  });
  return required(data, 'repositories') as unknown as RepositoryList;
}

export async function getRepository(repoId: string, opts: Signal = {}): Promise<Repository> {
  const { data } = await api.GET('/api/v1/repositories/{repoId}', {
    params: { path: { repoId } },
    signal: opts.signal,
  });
  return required(data, 'repository') as unknown as Repository;
}

export async function getRepositoryStatus(
  repoId: string,
  opts: Signal = {},
): Promise<RepositoryStatus> {
  const { data } = await api.GET('/api/v1/repositories/{repoId}/status', {
    params: { path: { repoId } },
    signal: opts.signal,
  });
  return required(data, 'repository status') as unknown as RepositoryStatus;
}

export async function enableRepository(input: {
  installation_id: string;
  full_name: string;
}): Promise<Repository> {
  const { data } = await api.POST('/api/v1/repositories', { body: input });
  return required(data, 'repository') as unknown as Repository;
}

export async function initializeRepository(repoId: string): Promise<IndexRequest> {
  const { data } = await api.POST('/api/v1/repositories/{repoId}/initialize', {
    params: { path: { repoId } },
  });
  return required(data, 'initialize');
}

export async function rebuildRepositoryGraph(repoId: string): Promise<IndexRequest> {
  const { data } = await api.POST('/api/v1/repositories/{repoId}/graph/rebuild', {
    params: { path: { repoId } },
  });
  return required(data, 'rebuild');
}

export async function updateRepositorySettings(
  repoId: string,
  patch: RepositorySettingsPatch,
): Promise<RepositorySettings> {
  const { data } = await api.PATCH('/api/v1/repositories/{repoId}/settings', {
    params: { path: { repoId } },
    body: patch,
  });
  return required(data, 'repository settings');
}

/** PENDING (assumed route): installation repositories, flagged when already enabled. */
export async function listInstallationRepositories(
  organizationId: string,
  opts: Signal = {},
): Promise<InstallationRepository[]> {
  const { data } = await api.GET('/api/v1/installations/repositories', {
    params: { query: { organization_id: organizationId } },
    signal: opts.signal,
  });
  return required(data, 'installation repositories').items;
}

/** PENDING (assumed route): top risk areas of a repository. */
export async function listRiskAreas(repoId: string, opts: Signal = {}): Promise<RiskArea[]> {
  const { data } = await api.GET('/api/v1/repositories/{repoId}/risk-areas', {
    params: { path: { repoId } },
    signal: opts.signal,
  });
  return required(data, 'risk areas').items;
}

// ---------------------------------------------------------------------------------------------
// Pull requests and reviews (API-009, pending)
// ---------------------------------------------------------------------------------------------

/** Organization-wide when `query.repository_id` is unset, else the repository route. */
export async function listPullRequests(
  query: PullRequestListQuery,
  opts: Signal = {},
): Promise<Page<PullRequestSummary>> {
  const { repository_id, organization_id, ...rest } = query;
  if (repository_id) {
    const { data } = await api.GET('/api/v1/repositories/{repoId}/pull-requests', {
      params: { path: { repoId: repository_id }, query: rest },
      signal: opts.signal,
    });
    return required(data, 'pull requests');
  }
  const { data } = await api.GET('/api/v1/pull-requests', {
    params: { query: { organization_id, ...rest } },
    signal: opts.signal,
  });
  return required(data, 'pull requests');
}

export async function triggerManualReview(prId: string): Promise<ManualReviewResponse> {
  const { data } = await api.POST('/api/v1/pull-requests/{prId}/review', {
    params: { path: { prId } },
  });
  return required(data, 'manual review');
}

export async function getReview(reviewId: string, opts: Signal = {}): Promise<ReviewDetail> {
  const { data } = await api.GET('/api/v1/reviews/{reviewId}', {
    params: { path: { reviewId } },
    signal: opts.signal,
  });
  return required(data, 'review');
}

export async function listReviewHistory(
  prId: string,
  opts: Signal = {},
): Promise<ReviewRunSummary[]> {
  const { data } = await api.GET('/api/v1/pull-requests/{prId}/reviews', {
    params: { path: { prId } },
    signal: opts.signal,
  });
  return required(data, 'review history').items;
}

// ---------------------------------------------------------------------------------------------
// Findings (API-010, pending)
// ---------------------------------------------------------------------------------------------

export async function listReviewFindings(
  reviewId: string,
  state: FindingStateFilter = 'all',
  opts: Signal = {},
): Promise<FindingSummary[]> {
  const { data } = await api.GET('/api/v1/reviews/{reviewId}/findings', {
    params: { path: { reviewId }, query: { state } },
    signal: opts.signal,
  });
  return required(data, 'findings').items;
}

export async function getFinding(findingId: string, opts: Signal = {}): Promise<FindingDetail> {
  const { data } = await api.GET('/api/v1/findings/{findingId}', {
    params: { path: { findingId } },
    signal: opts.signal,
  });
  return required(data, 'finding');
}

export async function getFindingTrace(findingId: string, opts: Signal = {}): Promise<FindingTrace> {
  const { data } = await api.GET('/api/v1/findings/{findingId}/trace', {
    params: { path: { findingId } },
    signal: opts.signal,
  });
  return required(data, 'finding trace');
}

// ---------------------------------------------------------------------------------------------
// Graph proxy and source (API-011, pending)
// ---------------------------------------------------------------------------------------------

/** Redacted excerpt; the API caps it at 200 lines. */
export async function getSourceExcerpt(
  repoId: string,
  query: { path: string; start: number; end: number; snapshot?: string },
  opts: Signal = {},
): Promise<SourceExcerpt> {
  const { data } = await api.GET('/api/v1/repositories/{repoId}/source', {
    params: { path: { repoId }, query },
    signal: opts.signal,
  });
  return required(data, 'source excerpt');
}

/** PENDING (API-009, assumed route): last review and open PR counts per repository. */
export async function listRepositoryActivity(
  organizationId: string,
  opts: Signal = {},
): Promise<RepositoryActivity[]> {
  const { data } = await api.GET('/api/v1/organizations/{id}/repository-activity', {
    params: { path: { id: organizationId } },
    signal: opts.signal,
  });
  return required(data, 'repository activity').items;
}
