import type { FindingSummary, ReviewDetail } from '../lib/api/pending';

export const REVIEW_ID = 'run-151';

export function reviewDetail(overrides: Partial<ReviewDetail> = {}): ReviewDetail {
  return {
    id: REVIEW_ID,
    state: 'COMPLETED',
    degraded: true,
    stage: null,
    created_at: '2026-10-07T10:00:00Z',
    finished_at: '2026-10-07T10:02:30Z',
    pull_request: {
      id: 'pr-151',
      number: 151,
      title: 'Allow admins to update users',
      author: 'octocat',
      url: 'https://github.com/acme/api/pull/151',
      repository_id: 'repo-1',
      repository_full_name: 'acme/api',
      base_ref: 'main',
      head_ref: 'feature/admin-update',
      base_sha: 'aaaaaaa0000000',
      head_sha: 'bbbbbbb1111111',
    },
    trigger: 'webhook',
    degraded_reasons: ['performance reviewer timed out'],
    failure: null,
    stages: [
      {
        name: 'indexing',
        state: 'succeeded',
        started_at: '2026-10-07T10:00:00Z',
        finished_at: '2026-10-07T10:00:20Z',
        duration_ms: 20_000,
      },
      {
        name: 'reviewing',
        state: 'succeeded',
        started_at: '2026-10-07T10:00:20Z',
        finished_at: '2026-10-07T10:02:00Z',
        duration_ms: 100_000,
      },
    ],
    reviewer_runs: [
      {
        reviewer: 'security',
        version: 'v1',
        state: 'succeeded',
        prompt_version: 'p3',
        model: 'tier-2',
        duration_ms: 40_000,
        error_class: null,
        findings: 2,
      },
      {
        reviewer: 'performance',
        version: 'v1',
        state: 'timed_out',
        prompt_version: 'p1',
        model: 'tier-1',
        duration_ms: 60_000,
        error_class: 'transient',
        findings: 0,
      },
    ],
    completeness: {
      reviewers_planned: ['security', 'performance'],
      reviewers_succeeded: ['security'],
      reviewers_failed: [{ reviewer: 'performance', reason: 'timed out' }],
      not_executed: [{ check: 'reproduction', reason: 'no test runner configured' }],
    },
    risk: {
      level: 'high',
      score: 0.82,
      signals: [{ name: 'auth_path', weight: 0.4, detail: 'touches AuthService.authorize' }],
      effects: {
        reviewers: ['security', 'performance'],
        depth: 'deep',
        token_budget: 120_000,
        model_call_budget: 12,
      },
    },
    change_summary: {
      files: [
        {
          path: 'src/admin/admin.service.ts',
          status: 'modified',
          old_path: null,
          additions: 12,
          deletions: 4,
        },
        {
          path: 'src/users/user.controller.ts',
          status: 'modified',
          old_path: null,
          additions: 3,
          deletions: 1,
        },
      ],
      behavioral_symbols: [
        {
          key: 'k-update',
          name: 'AdminService.updateUser',
          kind: 'method',
          path: 'src/admin/admin.service.ts',
          change: 'modified',
        },
      ],
      api_contracts: [],
      dependencies: [],
      schemas: [],
    },
    coverage: { reviewed_clusters: 3, unreviewed_clusters: [] },
    counts_by_state: { PUBLISHED: 2, SUPPRESSED_LOW_CONFIDENCE: 1, SUPPRESSED_DUPLICATE: 1 },
    trace_id: '4bf92f3577b34da6a3ce929d0e0e4736',
    ...overrides,
  };
}

export function finding(overrides: Partial<FindingSummary> = {}): FindingSummary {
  return {
    id: 'f-1',
    review_id: REVIEW_ID,
    title: 'Permission check removed before updating a user',
    severity: 'critical',
    category: 'security',
    reviewer: 'security',
    reviewer_version: 'v1',
    state: 'PUBLISHED',
    confidence: 0.91,
    anchor: {
      path: 'src/admin/admin.service.ts',
      start_line: 42,
      end_line: 48,
      symbol_key: 'k-update',
    },
    relocated: false,
    suppression_reason: null,
    evidence_summary: 'call to PermissionService.check() removed',
    evidence_count: 4,
    ...overrides,
  };
}

export const reviewFindings: FindingSummary[] = [
  finding(),
  finding({
    id: 'f-2',
    title: 'Controller bypasses the guard',
    severity: 'high',
    relocated: true,
    anchor: { path: 'src/users/user.controller.ts', start_line: 7, end_line: 7, symbol_key: null },
  }),
  finding({
    id: 'f-3',
    title: 'Possible N+1 query',
    severity: 'low',
    state: 'SUPPRESSED_LOW_CONFIDENCE',
    confidence: 0.3,
    suppression_reason: 'confidence 0.30 below 0.60',
  }),
  finding({
    id: 'f-4',
    title: 'Same as f-1',
    severity: 'critical',
    state: 'SUPPRESSED_DUPLICATE',
    suppression_reason: 'merged into f-1',
  }),
];
