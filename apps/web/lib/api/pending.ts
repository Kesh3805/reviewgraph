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
// WEB-004: repository intelligence (`GET /repositories/:id/intelligence`, read from
// `snapshots.stats`) and the PROF-001 profile (`GET /repositories/:id/profile` exists but its
// body is untyped in the OpenAPI document).
// ---------------------------------------------------------------------------------------------

export interface SnapshotInfo {
  id: string;
  kind: 'full' | 'delta';
  commit_sha: string;
  branch: string | null;
  /** Deltas since the last full snapshot (0 for a full one). */
  chain_length: number;
  created_at: string;
  node_count: number;
  edge_count: number;
}

export interface GraphStats {
  nodes_by_kind: Record<string, number>;
  edges_by_kind: Record<string, number>;
  unresolved_references: number;
  parse_failures: number;
  /** 10 buckets over [0, 1): bucket i counts edges with confidence in [i/10, (i+1)/10). */
  edge_confidence_histogram: number[];
  resolved_by: Record<string, number>;
}

export interface RepositoryIntelligence {
  snapshots: SnapshotInfo[];
  fingerprint: string | null;
  versions: {
    tool_version: string | null;
    facts_schema_version: number | null;
    analyzers: Record<string, string>;
  };
  /** Stats of the latest snapshot; null before the first index. */
  stats: GraphStats | null;
  languages: { name: string; files: number; share: number }[];
  frameworks: string[];
  index_jobs: {
    id: string;
    kind: 'initialize' | 'rebuild' | 'incremental';
    state: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';
    created_at: string;
    finished_at: string | null;
    error_class: ErrorClass | null;
  }[];
}

export interface Convention {
  id: string;
  rule: string;
  scope: string;
  samples: number;
  violations: number;
  /** 0..1 */
  consistency: number;
  /** 0..1 */
  confidence: number;
  /** PROF-003: confidence ≥ 0.9 and samples ≥ 10; computed client-side when absent. */
  enforceable?: boolean;
  exceptions: { scope: string; reason: string | null }[];
  source: 'inferred' | 'documented' | 'explicit';
}

/** The subset of PROF-001 `RepositoryProfile` the profile page renders. */
export interface RepositoryProfile {
  repository_id: string;
  snapshot_id: string;
  profile_version: number;
  config_hash: string;
  computed_at: string;
  architecture: {
    layers: { name: string; globs: string[]; member_count: number; role_confidence: number }[];
    /** Observed layer dependency counts (IMPORTS/CALLS). */
    matrix: { from: string; to: string; count: number }[];
  };
  conventions: Convention[];
  documentation_rules: { id: string; title: string; path: string; topics: string[] }[];
  /** Effective policy per topic (POL-004), when the API includes it. */
  effective_policy?: EffectivePolicy[];
}

// ---------------------------------------------------------------------------------------------
// WEB-008: rules and suppressions (POL-001..POL-006, assumed routes), integrations (GH-010,
// GH-012), usage (GW accounting, QB-005) and organization settings (SEC-007).
// ---------------------------------------------------------------------------------------------

/** `GET /repositories/:id/rules`: the effective `.review/config.yaml` and explicit rules. */
export interface RepositoryRules {
  config: { path: string; yaml: string | null };
  rules: { id: string; description: string; severity: Severity; violations_30d: number }[];
}

export interface Suppression {
  id: string;
  kind: 'fingerprint' | 'symbol' | 'path' | 'rule';
  value: string;
  reason: string;
  created_by: string;
  created_at: string;
  revoked_at: string | null;
  revoked_by: string | null;
}

export interface AuditEntry {
  id: string;
  action: string;
  actor: string;
  target: string;
  at: string;
}

export interface CreateSuppressionInput {
  kind: Suppression['kind'];
  value: string;
  reason: string;
}

/** `GET /organizations/:id/integrations/github`. */
export interface GithubIntegration {
  installation: {
    id: string;
    account_login: string;
    state: 'active' | 'suspended' | 'deleted';
    installed_at: string;
  } | null;
  /** GH-010: the App permission manifest check (no merge permission, minimal scopes). */
  permissions_check: {
    ok: boolean;
    checked_at: string | null;
    missing: string[];
    excess: string[];
  };
  webhooks: {
    last_delivery_at: string | null;
    last_delivery_event: string | null;
    deliveries_24h: number;
    signature_failures_24h: number;
  };
  /** GH-012 polling reconciler. */
  reconciler: {
    state: 'ok' | 'lagging' | 'failing' | 'disabled';
    last_run_at: string | null;
    repaired_24h: number;
  };
}

export type UsageGroupBy = 'day' | 'reviewer' | 'tier' | 'model';

export interface UsageMetrics {
  model_calls: number;
  input_tokens: number;
  output_tokens: number;
  cached_tokens: number;
  cost_usd_micros: number;
}

/** `GET /organizations/:id/usage?from&to&group_by`. */
export interface UsageReport {
  from: string;
  to: string;
  group_by: UsageGroupBy;
  rows: (UsageMetrics & { key: string })[];
  totals: UsageMetrics & { reviewed_prs: number; useful_findings: number };
  cost_per_reviewed_pr_usd_micros: number | null;
  /** QB-005. */
  cost_per_useful_finding_usd_micros: number | null;
}

/** `GET/PATCH /organizations/:id/settings`; PATCH carries `updated_at` as a precondition. */
export interface OrganizationSettings {
  retention: { source_days: number; artifacts_days: number };
  /** When false, no source is sent to external model providers (next runs). */
  external_models: boolean;
  updated_at: string;
}

export interface OrganizationSettingsPatch {
  retention?: OrganizationSettings['retention'];
  external_models?: boolean;
  /** Optimistic concurrency: 409 when the settings changed since this timestamp. */
  updated_at: string;
}

export interface Member {
  user_id: string;
  login: string;
  display_name: string | null;
  role: 'owner' | 'admin' | 'member' | 'viewer';
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

export type ReviewerType =
  'correctness' | 'security' | 'test' | 'architecture' | 'performance' | 'maintainability';
export type ReviewerRunState =
  'pending' | 'running' | 'succeeded' | 'failed' | 'skipped' | 'timed_out';

/** `GET /pull-requests/:id/reviews` items (the history selector). */
export interface ReviewRunSummary extends RunRef {
  head_sha: string;
  trigger: 'webhook' | 'manual' | 'reconciler';
  finished_at: string | null;
  superseded_by: string | null;
}

export interface StageTiming {
  name: string;
  state: 'pending' | 'running' | 'succeeded' | 'failed' | 'skipped';
  started_at: string | null;
  finished_at: string | null;
  duration_ms: number | null;
}

export interface ReviewerRun {
  reviewer: ReviewerType;
  version: string;
  state: ReviewerRunState;
  prompt_version: string;
  model: string | null;
  duration_ms: number | null;
  error_class: ErrorClass | null;
  findings: number;
}

/** INV-013: computed from `reviewer_runs` rows, never self-reported. */
export interface Completeness {
  reviewers_planned: ReviewerType[];
  reviewers_succeeded: ReviewerType[];
  reviewers_failed: { reviewer: ReviewerType; reason: string }[];
  /** Checks that did not run (INV-014), each with the reason. */
  not_executed: { check: string; reason: string }[];
}

/** The CHG change model summary (a `stage_outputs` JSON contract). */
export interface ChangeSummary {
  files: {
    path: string;
    status: 'added' | 'modified' | 'deleted' | 'renamed';
    old_path: string | null;
    additions: number;
    deletions: number;
  }[];
  behavioral_symbols: {
    key: string;
    name: string;
    kind: string;
    path: string;
    change: 'added' | 'modified' | 'removed' | 'signature_changed';
  }[];
  api_contracts: { name: string; change: string }[];
  dependencies: { name: string; from: string | null; to: string | null }[];
  schemas: { name: string; change: string }[];
}

/** The RISK assessment (a `stage_outputs` JSON contract). */
export interface RiskAssessment {
  level: 'low' | 'medium' | 'high' | 'critical';
  /** 0..1 */
  score: number;
  signals: { name: string; weight: number; detail: string | null }[];
  effects: {
    reviewers: ReviewerType[];
    depth: string;
    token_budget: number | null;
    model_call_budget: number | null;
  };
}

/** `GET /reviews/:id` (assumed alias of `GET /pull-requests/:id/reviews/:reviewId`). */
export interface ReviewDetail extends RunRef {
  pull_request: {
    id: string;
    number: number;
    title: string;
    author: string;
    url: string | null;
    repository_id: string;
    repository_full_name: string;
    base_ref: string;
    head_ref: string;
    base_sha: string;
    head_sha: string;
  };
  trigger: ReviewRunSummary['trigger'];
  finished_at: string | null;
  degraded_reasons: string[];
  /** Set when the run failed: the stage and error class only, never a stack trace. */
  failure: { stage: string; error_class: ErrorClass } | null;
  stages: StageTiming[];
  reviewer_runs: ReviewerRun[];
  completeness: Completeness;
  risk: RiskAssessment | null;
  change_summary: ChangeSummary | null;
  coverage: {
    reviewed_clusters: number;
    unreviewed_clusters: { id: string; reason: string; files: string[] }[];
  } | null;
  counts_by_state: Partial<Record<FindingState, number>>;
  trace_id: string | null;
}

// ---------------------------------------------------------------------------------------------
// API-010: findings
// ---------------------------------------------------------------------------------------------

export type FindingState =
  | 'GENERATED'
  | 'EVIDENCE_COLLECTED'
  | 'VERIFIED'
  | 'DEDUPLICATED'
  | 'PRIORITIZED'
  | 'PUBLISHED'
  | 'SUPPRESSED_LOW_CONFIDENCE'
  | 'SUPPRESSED_DUPLICATE'
  | 'SUPPRESSED_PREEXISTING'
  | 'SUPPRESSED_NOT_ACTIONABLE'
  | 'SUPPRESSED_POLICY'
  | 'INVALIDATED';

export interface FindingAnchor {
  path: string;
  start_line: number;
  end_line: number;
  symbol_key: string | null;
}

export interface FindingSummary {
  id: string;
  review_id: string;
  title: string;
  severity: Severity;
  category: string;
  reviewer: ReviewerType;
  reviewer_version: string;
  state: FindingState;
  /** 0..1 */
  confidence: number;
  anchor: FindingAnchor;
  /** Anchored outside the diff (relocated to the summary). */
  relocated: boolean;
  /** Set for suppressed findings. */
  suppression_reason: string | null;
  evidence_summary: string | null;
  evidence_count: number;
}

export type FindingStateFilter = 'published' | 'verified' | 'suppressed' | 'all';

/** ADR-011 confidence terms. */
export type ConfidenceTerm =
  | 'anchor'
  | 'deterministic'
  | 'graph'
  | 'repo'
  | 'reproduction'
  | 'agreement'
  | 'contradiction'
  | 'uncertainty';

export interface ConfidenceComponent {
  term: ConfidenceTerm;
  /** The component score in [0, 1]. */
  value: number;
  /** Signed weight from the `verification_version` table (negative for penalties). */
  weight: number;
}

export interface Publication {
  state: 'published' | 'not_published' | 'resolved';
  provider_comment_url: string | null;
  published_at: string | null;
}

/** `GET /findings/:id` (API-010, schema `FindingDetail`). */
export interface FindingDetail extends FindingSummary {
  explanation: string;
  symbols: { key: string; name: string; kind: string }[];
  confidence_components: ConfidenceComponent[];
  verification_version: string;
  repository_id: string;
  pull_request: { id: string; number: number; title: string; repository_full_name: string };
  /** Snapshot ids used for source excerpts (base is null for a file added in the PR). */
  snapshots: { base: string | null; head: string };
  publication: Publication | null;
}

/** A typed evidence item (DOM-007); code is referenced, never embedded. */
export interface EvidenceItem {
  kind: string;
  summary: string;
  path: string | null;
  start_line: number | null;
  end_line: number | null;
  snapshot_id: string | null;
}

export type PolicySourceKind = 'explicit' | 'documented' | 'convention' | 'generic';

export interface PolicySource {
  kind: PolicySourceKind;
  /** Rule, document or convention id; null for generic guidance. */
  id: string | null;
  confidence?: number | null;
  samples?: number | null;
}

/** POL-004 `EffectivePolicy`. */
export interface EffectivePolicy {
  topic: string;
  decision: 'required' | 'forbidden' | 'allowed' | 'no_policy';
  winner: PolicySource;
  overridden: PolicySource[];
  conflict?: boolean;
}

export interface AnchorSide {
  path: string;
  start_line: number;
  end_line: number;
  snapshot_id: string;
  /** The finding predicate evaluated on this side (null when it could not be evaluated). */
  predicate: { name: string; holds: boolean | null };
}

/** `GET /findings/:id/trace` (API-010, schema `FindingTrace`). No prompts or model output. */
export interface FindingTrace {
  finding_id: string;
  /** True when some stage rows are missing (an older run). */
  incomplete: boolean;
  change: { path: string; status: string } | null;
  symbol: { key: string; name: string; kind: string } | null;
  context_items: { kind: string; ref: string }[];
  reviewer: { name: string; version: string };
  candidate: { id: string; created_at: string } | null;
  /** Curated path from the entrypoint to the changed symbol (at most 30 nodes). */
  impact_path: GraphPath | null;
  verification: {
    stage: string;
    outcome: 'passed' | 'failed' | 'inconclusive' | 'not_executed';
    evidence: EvidenceItem[];
  }[];
  base_head: { symbol_key: string; base: AnchorSide | null; head: AnchorSide } | null;
  dedup_merges: { finding_id: string; reviewer: string; title: string; similarity: number }[];
  effective_policy: EffectivePolicy[];
  publication: Publication | null;
}

// ---------------------------------------------------------------------------------------------
// API-012: feedback
// ---------------------------------------------------------------------------------------------

export type Verdict =
  'useful' | 'false_positive' | 'already_handled' | 'not_relevant' | 'intentional';

export type SuppressionKind = 'fingerprint' | 'symbol' | 'path';

export interface FeedbackInput {
  verdict: Verdict;
  /** Plain text, at most 2,000 characters. */
  comment?: string;
  /** Only with `intentional` or `not_relevant`; requires maintainer. */
  create_suppression?: { kind: SuppressionKind; reason: string };
}

/** `GET /findings/:id/feedback` (assumed shape): the caller's verdict plus aggregate counts. */
export interface FindingFeedback {
  mine: { verdict: Verdict; comment: string | null; updated_at: string } | null;
  counts: Record<Verdict, number>;
}

// ---------------------------------------------------------------------------------------------
// API-011: graph proxy and source excerpts
// ---------------------------------------------------------------------------------------------

export interface GraphNode {
  key: string;
  name: string;
  qualified_name?: string | null;
  kind: string;
  path: string | null;
  line: number | null;
}

export interface GraphEdge {
  source: string;
  target: string;
  kind: string;
  /** 0..1 */
  confidence: number;
}

/** An ordered chain: `nodes[0]` is the entrypoint, `edges[i]` joins `nodes[i]` → `nodes[i+1]`. */
export interface GraphPath {
  nodes: GraphNode[];
  edges: GraphEdge[];
  min_confidence?: number;
}

/** Symbol kinds the explorer filters on (GX-001). */
export type SymbolKindFilter =
  'class' | 'method' | 'function' | 'endpoint' | 'queue' | 'table' | 'test';

/** `GET /repositories/:id/graph/symbols?q&kind&snapshot` items (API-011 → API-013). */
export interface SymbolSearchResult {
  key: string;
  name: string;
  qualified_name: string;
  kind: string;
  path: string;
  line: number;
  in_degree: number;
  out_degree: number;
}

export interface SymbolSearchResponse {
  items: SymbolSearchResult[];
  snapshot_id: string;
  truncated: boolean;
}

/** `GET /repositories/:id/graph/symbols/:key` — the CLI-007 `review graph symbol` fields. */
export interface SymbolDetail {
  key: string;
  id: string;
  name: string;
  qualified_name: string;
  kind: string;
  path: string;
  start_line: number;
  end_line: number;
  signature: string | null;
  visibility: string | null;
  framework_facts: { name: string; value: string }[];
  /** Earlier keys of the same symbol (renames and moves, SID lineage). */
  lineage: { key: string; snapshot_id: string; change: string }[];
  edges_in: Record<string, number>;
  edges_out: Record<string, number>;
  /** 10 buckets of incident edge confidence. */
  confidence_histogram: number[];
  snapshot_id: string;
}

/** `GET /repositories/:id/source` — redacted and capped at 200 lines. */
export interface SourceExcerpt {
  path: string;
  start_line: number;
  end_line: number;
  snapshot_id: string;
  language: string | null;
  text: string;
  truncated: boolean;
  redacted: boolean;
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
  /** API-008 route; PROF-001 body typed here (404 before the first index). */
  '/api/v1/repositories/{repoId}/profile': {
    get: GetOp<RepositoryProfile, PathParams<'repoId'>>;
  };
  /** WEB-004 (API side not built yet). */
  '/api/v1/repositories/{repoId}/intelligence': {
    get: GetOp<RepositoryIntelligence, PathParams<'repoId'>>;
  };
  /** WEB-008 (assumed routes): rules, suppressions and their audit history. */
  '/api/v1/repositories/{repoId}/rules': { get: GetOp<RepositoryRules, PathParams<'repoId'>> };
  '/api/v1/repositories/{repoId}/suppressions': {
    get: GetOp<{ items: Suppression[]; audit: AuditEntry[] }, PathParams<'repoId'>>;
    post: BodyOp<Suppression, CreateSuppressionInput, PathParams<'repoId'>>;
  };
  '/api/v1/repositories/{repoId}/suppressions/{suppressionId}': {
    delete: BodyOp<Suppression, never, { path: { repoId: string; suppressionId: string } }>;
  };
  /** WEB-008. */
  '/api/v1/organizations/{id}/integrations/github': {
    get: GetOp<GithubIntegration, PathParams<'id'>>;
  };
  '/api/v1/organizations/{id}/usage': {
    get: GetOp<
      UsageReport,
      { path: { id: string }; query: { from: string; to: string; group_by: UsageGroupBy } }
    >;
  };
  '/api/v1/organizations/{id}/settings': {
    get: GetOp<OrganizationSettings, PathParams<'id'>>;
    patch: BodyOp<OrganizationSettings, OrganizationSettingsPatch, PathParams<'id'>>;
  };
  /** Assumed list route for the members table. */
  '/api/v1/organizations/{id}/members': { get: GetOp<{ items: Member[] }, PathParams<'id'>> };
  '/api/v1/organizations/{id}/members/{userId}': {
    patch: BodyOp<Member, { role: Member['role'] }, { path: { id: string; userId: string } }>;
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
  /** API-009: review history of a pull request. */
  '/api/v1/pull-requests/{prId}/reviews': {
    get: GetOp<{ items: ReviewRunSummary[] }, PathParams<'prId'>>;
  };
  /** API-009 (assumed alias): review detail by run id. */
  '/api/v1/reviews/{reviewId}': { get: GetOp<ReviewDetail, PathParams<'reviewId'>> };
  /** API-010. */
  '/api/v1/reviews/{reviewId}/findings': {
    get: GetOp<
      { items: FindingSummary[] },
      {
        path: { reviewId: string };
        query?: { state?: FindingStateFilter; severity?: Severity; reviewer?: string };
      }
    >;
  };
  /** API-010. */
  '/api/v1/findings/{findingId}': { get: GetOp<FindingDetail, PathParams<'findingId'>> };
  '/api/v1/findings/{findingId}/trace': { get: GetOp<FindingTrace, PathParams<'findingId'>> };
  /** API-012 (the POST response shape is assumed to equal the GET). */
  '/api/v1/findings/{findingId}/feedback': {
    get: GetOp<FindingFeedback, PathParams<'findingId'>>;
    post: BodyOp<FindingFeedback, FeedbackInput, PathParams<'findingId'>>;
  };
  /** API-011: symbol search (ranked exact > prefix > substring, limit 50). */
  '/api/v1/repositories/{repoId}/graph/symbols': {
    get: GetOp<
      SymbolSearchResponse,
      {
        path: { repoId: string };
        query: { q: string; kind?: string; snapshot?: string; limit?: number };
      }
    >;
  };
  '/api/v1/repositories/{repoId}/graph/symbols/{key}': {
    get: GetOp<
      SymbolDetail,
      { path: { repoId: string; key: string }; query?: { snapshot?: string } }
    >;
  };
  /** API-011. */
  '/api/v1/repositories/{repoId}/source': {
    get: GetOp<
      SourceExcerpt,
      {
        path: { repoId: string };
        query: { path: string; start: number; end: number; snapshot?: string };
      }
    >;
  };
}
