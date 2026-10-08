/**
 * PENDING CONTRACTS: hand-written request/response types for API routes that are not in the
 * generated OpenAPI document yet (`generated.ts`).
 *
 * Every section names the task that will publish the route. When it lands:
 *   1. `pnpm -F @reviewgraph/web gen:api`
 *   2. delete the route from `PendingPaths` (and the types it alone uses),
 *   3. re-point the exported type aliases at `components['schemas'][...]` in `generated.ts`.
 * Shapes follow the task specs in `docs/planning/tasks/P27-P29-*.md` and `P30-P37-*.md`; fields
 * the specs leave open are marked "assumed".
 */
import type { DashboardSummary } from '../dashboard';
import type { Session } from '../session';
import type { components } from './generated';

// ---------------------------------------------------------------------------------------------
// openapi-fetch operation helpers
// ---------------------------------------------------------------------------------------------

type Json<T> = { content: { 'application/json': T } };

/** A read operation. `P` holds the `path` / `query` parameters. */
export interface GetOp<Res, P = { query?: never; path?: never }> {
  parameters: P;
  requestBody?: never;
  responses: { 200: Json<Res> };
}

/** A mutation with a JSON body (or none when `Body` is `never`). */
export type BodyOp<Res, Body, P = { query?: never; path?: never }> = {
  parameters: P;
  responses: { 200: Json<Res> };
} & ([Body] extends [never] ? { requestBody?: never } : { requestBody: Json<Body> });

type PathParams<K extends string> = { path: Record<K, string>; query?: never };

// ---------------------------------------------------------------------------------------------
// Shared vocabulary (mirrors packages/contracts/src/generated, kept local because the web app
// does not build the contracts package)
// ---------------------------------------------------------------------------------------------

export type Severity = 'critical' | 'high' | 'medium' | 'low' | 'info';
export type ErrorClass =
  | 'invalid_input'
  | 'not_found'
  | 'conflict'
  | 'transient'
  | 'rate_limited'
  | 'permanent'
  | 'cancelled'
  | 'internal';

export interface Page<T> {
  items: T[];
  /** Opaque; pass it back as `cursor`. Null on the last page. */
  next_cursor: string | null;
}

// ---------------------------------------------------------------------------------------------
// API-008 corrections. The generated document renders top-level nullable strings as `string[]`
// (a nestjs-zod OpenAPI 3.0 quirk), so these types restate the zod DTOs in
// apps/api/src/repositories/dto/repository.dto.ts.
// ---------------------------------------------------------------------------------------------

type Schemas = components['schemas'];

export type Repository = Omit<Schemas['RepositoryDto'], 'primary_language' | 'initialized_at'> & {
  primary_language: string | null;
  initialized_at: string | null;
};
export type RepositoryList = Page<Repository>;
export type RepositoryStatus = Omit<Schemas['RepositoryStatusDto'], 'profile_computed_at'> & {
  profile_computed_at: string | null;
};
export type RepositorySettings = Schemas['RepositorySettingsDto'];
export type RepositorySettingsPatch = Schemas['UpdateRepositorySettingsDto'];
export type IndexRequest = Schemas['IndexRequestDto'];
export type IndexState = RepositoryStatus['index_state'];

// ---------------------------------------------------------------------------------------------
// WEB-003: installation repositories for the "Add repository" dialog. No task specifies this
// route yet (assumed): it lists the repositories the organization's GitHub App installations can
// see, flagged when already enabled.
// ---------------------------------------------------------------------------------------------

export interface InstallationRepository {
  installation_id: string;
  full_name: string;
  private: boolean;
  /** True when the repository is already enabled for review. */
  enabled: boolean;
}

/**
 * Optional per-repository activity counters for the repositories table (assumed, API-009):
 * absent until the reviews tables exist; the table then shows a dash.
 */
export interface RepositoryActivity {
  repository_id: string;
  last_review_at: string | null;
  open_pull_requests: number;
}

/**
 * Top risk areas of a repository (assumed route, fed by RISK/finding history): the paths with
 * the most high-severity published findings over the window.
 */
export interface RiskArea {
  path: string;
  /** 0..1 */
  score: number;
  findings: number;
  top_severity: Severity;
}

// ---------------------------------------------------------------------------------------------
// API-009: pull requests and review runs
// ---------------------------------------------------------------------------------------------

/** `ReviewState` from packages/contracts (the PIPE state machine). */
export type ReviewState =
  | 'RECEIVED'
  | 'INDEXING'
  | 'ANALYZING'
  | 'REVIEWING'
  | 'VERIFYING'
  | 'PUBLISHING'
  | 'COMPLETED'
  | 'FAILED_INDEXING'
  | 'FAILED_ANALYSIS'
  | 'FAILED_REVIEW'
  | 'FAILED_PUBLISH'
  | 'SUPERSEDED'
  | 'CANCELLED';

export interface RunRef {
  id: string;
  state: ReviewState;
  /** A COMPLETED run with failed or skipped parts (PIPE-008). */
  degraded: boolean;
  stage: string | null;
  created_at: string;
}

export interface PullRequestSummary {
  id: string;
  repository_id: string;
  repository_full_name: string;
  number: number;
  title: string;
  author: string;
  state: 'open' | 'closed';
  draft: boolean;
  head_sha: string;
  url: string | null;
  latest_run: RunRef | null;
  /** Published findings of the latest run. */
  findings_by_severity: Record<Severity, number>;
  updated_at: string;
}

export interface PullRequestListQuery {
  /** Organization-wide list only (assumed route `GET /pull-requests`). */
  organization_id?: string;
  repository_id?: string;
  state?: 'open' | 'closed';
  has_findings?: boolean;
  severity?: Severity;
  cursor?: string;
  limit?: number;
}

/** Response of `POST /pull-requests/:id/review` (manual trigger). */
export interface ManualReviewResponse {
  review_id: string;
  /** False when the per-minute idempotency key matched an existing run. */
  created: boolean;
}

// ---------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------

export interface PendingPaths {
  /** API-004: the response body is not described in the OpenAPI document. */
  '/api/v1/auth/me': { get: GetOp<Session> };
  /** WEB-002 (deferred API half): organization dashboard. */
  '/api/v1/organizations/{id}/dashboard': { get: GetOp<DashboardSummary, PathParams<'id'>> };
  /** WEB-003 (assumed): repositories visible to the organization's installations. */
  '/api/v1/installations/repositories': {
    get: GetOp<{ items: InstallationRepository[] }, { query: { organization_id: string } }>;
  };
  /** API-009 (assumed): per-repository review activity for the repositories table. */
  '/api/v1/organizations/{id}/repository-activity': {
    get: GetOp<{ items: RepositoryActivity[] }, PathParams<'id'>>;
  };
  /** Assumed route: top risk areas for the repository overview. */
  '/api/v1/repositories/{repoId}/risk-areas': {
    get: GetOp<{ items: RiskArea[] }, PathParams<'repoId'>>;
  };
  /** API-009. */
  '/api/v1/repositories/{repoId}/pull-requests': {
    get: GetOp<
      Page<PullRequestSummary>,
      { path: { repoId: string }; query?: Omit<PullRequestListQuery, 'organization_id'> }
    >;
  };
  /** API-009 (assumed organization-wide variant for `/pull-requests`). */
  '/api/v1/pull-requests': {
    get: GetOp<Page<PullRequestSummary>, { query: PullRequestListQuery }>;
  };
  /** API-009: manual review at the current head (maintainer). */
  '/api/v1/pull-requests/{prId}/review': {
    post: BodyOp<ManualReviewResponse, never, PathParams<'prId'>>;
  };
}
