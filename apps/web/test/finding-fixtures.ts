import type { FindingDetail, FindingTrace, SourceExcerpt } from '../lib/api/pending';
import { finding } from './fixtures';

export const FINDING_ID = 'f-1';

export function findingDetail(overrides: Partial<FindingDetail> = {}): FindingDetail {
  return {
    ...finding(),
    explanation: 'AdminService.updateUser no longer calls PermissionService.check().',
    symbols: [{ key: 'k-update', name: 'AdminService.updateUser', kind: 'method' }],
    // 0.25*1 + 0.20*1 + 0.20*0.9 + 0.15*0.8 + 0.10*0 + 0.10*0.5 - 0.35*0 - 0.15*0.2 = 0.77
    confidence: 0.77,
    confidence_components: [
      { term: 'anchor', value: 1, weight: 0.25 },
      { term: 'deterministic', value: 1, weight: 0.2 },
      { term: 'graph', value: 0.9, weight: 0.2 },
      { term: 'repo', value: 0.8, weight: 0.15 },
      { term: 'reproduction', value: 0, weight: 0.1 },
      { term: 'agreement', value: 0.5, weight: 0.1 },
      { term: 'contradiction', value: 0, weight: -0.35 },
      { term: 'uncertainty', value: 0.2, weight: -0.15 },
    ],
    verification_version: 'ver-3',
    repository_id: 'repo-1',
    pull_request: {
      id: 'pr-151',
      number: 151,
      title: 'Allow admins',
      repository_full_name: 'acme/api',
    },
    snapshots: { base: 'snap-base', head: 'snap-head' },
    publication: {
      state: 'published',
      provider_comment_url: 'https://github.com/acme/api/pull/151#discussion_r1',
      published_at: '2026-10-07T10:02:30Z',
    },
    ...overrides,
  };
}

export function findingTrace(overrides: Partial<FindingTrace> = {}): FindingTrace {
  return {
    finding_id: FINDING_ID,
    incomplete: false,
    change: { path: 'src/admin/admin.service.ts', status: 'modified' },
    symbol: { key: 'k-authorize', name: 'AuthService.authorize', kind: 'method' },
    context_items: [{ kind: 'symbol', ref: 'k-update' }],
    reviewer: { name: 'security', version: 'v1' },
    candidate: { id: 'cand-1', created_at: '2026-10-07T10:01:00Z' },
    impact_path: {
      nodes: [
        {
          key: 'k-ctrl',
          name: 'update',
          qualified_name: 'UserController.update',
          kind: 'endpoint',
          path: 'src/users/user.controller.ts',
          line: 7,
        },
        {
          key: 'k-update',
          name: 'updateUser',
          qualified_name: 'AdminService.updateUser',
          kind: 'method',
          path: 'src/admin/admin.service.ts',
          line: 42,
        },
        {
          key: 'k-authorize',
          name: 'authorize',
          qualified_name: 'AuthService.authorize',
          kind: 'method',
          path: 'src/auth/auth.service.ts',
          line: 10,
        },
      ],
      edges: [
        { source: 'k-ctrl', target: 'k-update', kind: 'CALLS', confidence: 0.95 },
        { source: 'k-update', target: 'k-authorize', kind: 'CALLS', confidence: 0.55 },
      ],
    },
    verification: [
      {
        stage: 'anchor',
        outcome: 'passed',
        evidence: [
          {
            kind: 'anchor',
            summary: 'anchor lines exist at head',
            path: 'src/admin/admin.service.ts',
            start_line: 42,
            end_line: 48,
            snapshot_id: 'snap-head',
          },
        ],
      },
      { stage: 'reproduction', outcome: 'not_executed', evidence: [] },
    ],
    base_head: {
      symbol_key: 'k-update',
      base: {
        path: 'src/admin/admin.service.ts',
        start_line: 40,
        end_line: 50,
        snapshot_id: 'snap-base',
        predicate: { name: 'calls PermissionService.check', holds: true },
      },
      head: {
        path: 'src/admin/admin.service.ts',
        start_line: 40,
        end_line: 49,
        snapshot_id: 'snap-head',
        predicate: { name: 'calls PermissionService.check', holds: false },
      },
    },
    dedup_merges: [
      { finding_id: 'f-4', reviewer: 'correctness', title: 'Same as f-1', similarity: 0.93 },
    ],
    effective_policy: [
      {
        topic: 'GuardOnEndpoint',
        decision: 'required',
        winner: { kind: 'explicit', id: 'rule-guards' },
        overridden: [
          { kind: 'convention', id: 'endpoints_are_guarded', confidence: 0.96, samples: 40 },
        ],
      },
    ],
    publication: null,
    ...overrides,
  };
}

export function excerpt(snapshot: string, text: string, start = 40): SourceExcerpt {
  return {
    path: 'src/admin/admin.service.ts',
    start_line: start,
    end_line: start + text.split('\n').length - 1,
    snapshot_id: snapshot,
    language: 'typescript',
    text,
    truncated: false,
    redacted: true,
  };
}
