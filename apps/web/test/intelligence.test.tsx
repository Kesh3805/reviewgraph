// @vitest-environment jsdom
import './dom-stubs';
import { cleanup, screen, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { histogramBuckets } from '../components/intelligence/ConfidenceHistogram';
import { IntelligenceView } from '../components/intelligence/IntelligenceView';
import { isEnforceable } from '../components/profile/ConventionsTable';
import { ProfileView } from '../components/profile/ProfileView';
import type { RepositoryIntelligence, RepositoryProfile } from '../lib/api/pending';
import { problem, renderWithProviders, route } from './helpers';

const REPO = 'repo-1';

const intelligence: RepositoryIntelligence = {
  snapshots: [
    {
      id: 's2',
      kind: 'delta',
      commit_sha: 'bbbbbbb222',
      branch: 'main',
      chain_length: 1,
      created_at: '2026-10-07T10:00:00Z',
      node_count: 1210,
      edge_count: 4022,
    },
    {
      id: 's1',
      kind: 'full',
      commit_sha: 'aaaaaaa111',
      branch: 'main',
      chain_length: 0,
      created_at: '2026-10-06T10:00:00Z',
      node_count: 1200,
      edge_count: 4000,
    },
  ],
  fingerprint: 'fp-nestjs-layered',
  versions: {
    tool_version: '0.4.0',
    facts_schema_version: 2,
    analyzers: { 'ts-analyzer': '1.3.0' },
  },
  stats: {
    nodes_by_kind: { class: 200, method: 1000, endpoint: 10 },
    edges_by_kind: { CALLS: 3000, IMPORTS: 1022 },
    unresolved_references: 17,
    parse_failures: 2,
    edge_confidence_histogram: [0, 0, 1, 2, 3, 5, 8, 13, 400, 3590],
    resolved_by: { syntax: 3000, type_checker: 1000, heuristic: 22 },
  },
  languages: [{ name: 'TypeScript', files: 310, share: 0.97 }],
  frameworks: ['nestjs', 'typeorm'],
  index_jobs: [
    {
      id: 'job-1',
      kind: 'initialize',
      state: 'succeeded',
      created_at: '2026-10-06T09:59:00Z',
      finished_at: '2026-10-06T10:00:00Z',
      error_class: null,
    },
  ],
};

const profile: RepositoryProfile = {
  repository_id: REPO,
  snapshot_id: 's1',
  profile_version: 3,
  config_hash: 'cfg-abcdef123456',
  computed_at: '2026-10-06T10:01:00Z',
  architecture: {
    layers: [
      {
        name: 'controller',
        globs: ['src/**/*.controller.ts'],
        member_count: 8,
        role_confidence: 0.98,
      },
      {
        name: 'repository',
        globs: ['src/**/*.repository.ts'],
        member_count: 6,
        role_confidence: 0.95,
      },
    ],
    matrix: [{ from: 'controller', to: 'repository', count: 1 }],
  },
  conventions: [
    {
      id: 'controllers_do_not_access_repositories',
      rule: 'Controllers call services, not repositories',
      scope: 'src/**',
      samples: 8,
      violations: 1,
      consistency: 0.875,
      confidence: 0.86,
      exceptions: [],
      source: 'inferred',
    },
    {
      id: 'db_writes_use_transaction_wrapper',
      rule: 'Writes run in a transaction',
      scope: 'src/**/*.service.ts',
      samples: 24,
      violations: 1,
      consistency: 0.958,
      confidence: 0.93,
      exceptions: [{ scope: 'src/reports/**', reason: 'read replicas' }],
      source: 'inferred',
    },
    {
      id: 'endpoints_are_guarded',
      rule: 'Endpoints are guarded',
      scope: 'src/**',
      samples: 10,
      violations: 0,
      consistency: 1,
      confidence: 0.97,
      exceptions: [],
      source: 'inferred',
      enforceable: false,
    },
  ],
  documentation_rules: [
    {
      id: 'adr-7',
      title: 'Use guards on every endpoint',
      path: 'docs/adr/7.md',
      topics: ['GuardOnEndpoint'],
    },
  ],
  effective_policy: [
    {
      topic: 'TransactionOnWrite',
      decision: 'required',
      winner: { kind: 'explicit', id: 'rule-tx' },
      overridden: [
        {
          kind: 'convention',
          id: 'db_writes_use_transaction_wrapper',
          confidence: 0.93,
          samples: 24,
        },
        { kind: 'generic', id: null },
      ],
    },
  ],
};

const server = setupServer();
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
});
afterAll(() => server.close());

describe('repository intelligence and profile', () => {
  it('graph_stats_render', async () => {
    server.use(
      http.get(route('/repositories/:id/intelligence'), () => HttpResponse.json(intelligence)),
    );
    renderWithProviders(<IntelligenceView repoId={REPO} />);
    expect((await screen.findByTestId('stat-nodes')).textContent).toBe('1,210');
    expect(screen.getByTestId('stat-edges').textContent).toBe('4,022');
    expect(screen.getByTestId('stat-unresolved').textContent).toBe('17');
    expect(screen.getByTestId('stat-parse-failures').textContent).toBe('2');
    const resolved = screen.getByRole('table', { name: 'Resolved by' });
    expect(within(resolved).getByText('type_checker')).toBeTruthy();
    const histogram = screen.getByRole('table', { name: 'Edge confidence histogram' });
    expect(within(histogram).getAllByRole('row')).toHaveLength(11);
    expect(within(histogram).getByText('3590')).toBeTruthy();
    expect(histogramBuckets([1])[9]).toEqual({ bucket: '0.9–1.0', count: 0 });
    const snapshots = screen.getByRole('table', { name: 'Snapshots' });
    expect(within(snapshots).getByText('delta')).toBeTruthy();
    expect(screen.getByText('fp-nestjs-layered')).toBeTruthy();
  });

  it('conventions_table_marks_enforceable', async () => {
    server.use(http.get(route('/repositories/:id/profile'), () => HttpResponse.json(profile)));
    renderWithProviders(<ProfileView repoId={REPO} />);
    const tx = await screen.findByTestId('convention-db_writes_use_transaction_wrapper');
    expect(within(tx).getByText('Enforceable')).toBeTruthy();
    expect(within(tx).getByText('src/reports/**')).toBeTruthy();
    // Below the 0.9 confidence threshold.
    const ctrl = screen.getByTestId('convention-controllers_do_not_access_repositories');
    expect(within(ctrl).getByText('Informational')).toBeTruthy();
    // The API's explicit flag wins over the thresholds.
    const guards = screen.getByTestId('convention-endpoints_are_guarded');
    expect(within(guards).getByText('Informational')).toBeTruthy();
    expect(isEnforceable({ ...profile.conventions[1]!, samples: 9 })).toBe(false);

    const matrix = screen.getByRole('table', { name: 'Layer dependency matrix' });
    expect(within(matrix).getAllByText('1')).toHaveLength(1);
  });

  it('effective_policy_shows_overrides', async () => {
    server.use(http.get(route('/repositories/:id/profile'), () => HttpResponse.json(profile)));
    renderWithProviders(<ProfileView repoId={REPO} />);
    const policy = await screen.findByTestId('policy-TransactionOnWrite');
    expect(policy.textContent).toContain('Winner: explicit rule-tx');
    const struck = Array.from(policy.querySelectorAll('s')).map((s) => s.textContent);
    expect(struck).toEqual([
      'convention db_writes_use_transaction_wrapper (confidence 0.93, 24 samples)',
      'generic',
    ]);
  });

  it('empty_profile_state', async () => {
    server.use(
      http.get(route('/repositories/:id/profile'), () =>
        problem(404, 'the repository has no profile yet'),
      ),
    );
    renderWithProviders(<ProfileView repoId={REPO} />);
    expect(await screen.findByText('Profile is computed after the first full index.')).toBeTruthy();
  });
});
