// @vitest-environment jsdom
import './dom-stubs';
import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { GraphExplorer } from '../components/graph/GraphExplorer';
import { ImpactGroups } from '../components/graph/ImpactGroups';
import { PathFinder } from '../components/graph/PathFinder';
import type { GraphPath, ImpactGraph, PathResponse, SymbolSearchResult } from '../lib/api/pending';
import { renderWithProviders, route } from './helpers';

const nav = vi.hoisted(() => ({ search: '', replace: vi.fn() }));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ replace: nav.replace, push: vi.fn() }),
  usePathname: () => '/repositories/repo-1/graph',
  useSearchParams: () => new URLSearchParams(nav.search),
}));

const sym = (key: string, qualified: string): SymbolSearchResult => ({
  key,
  name: qualified.split('.').pop() ?? qualified,
  qualified_name: qualified,
  kind: 'method',
  path: 'src/x.ts',
  line: 1,
  in_degree: 1,
  out_degree: 1,
});
const SYMBOLS = [
  sym('k-ctrl', 'UserController.update'),
  sym('k-authorize', 'AuthService.authorize'),
];

const n = (key: string, q: string) => ({
  key,
  name: q,
  qualified_name: q,
  kind: 'method',
  path: null,
  line: null,
});
const direct: GraphPath = {
  nodes: [
    n('k-ctrl', 'UserController.update'),
    n('k-update', 'AdminService.updateUser'),
    n('k-authorize', 'AuthService.authorize'),
  ],
  edges: [
    { source: 'k-ctrl', target: 'k-update', kind: 'CALLS', confidence: 0.95 },
    { source: 'k-update', target: 'k-authorize', kind: 'CALLS', confidence: 0.55 },
  ],
  min_confidence: 0.55,
};
const viaGuard: GraphPath = {
  nodes: [
    n('k-ctrl', 'UserController.update'),
    n('k-guard', 'AdminGuard.canActivate'),
    n('k-authorize', 'AuthService.authorize'),
  ],
  edges: [
    { source: 'k-ctrl', target: 'k-guard', kind: 'GUARDED_BY', confidence: 0.9 },
    { source: 'k-guard', target: 'k-authorize', kind: 'CALLS', confidence: 0.8 },
  ],
  min_confidence: 0.8,
};

const server = setupServer();
let lastPathQuery: URLSearchParams | null = null;
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
  lastPathQuery = null;
  nav.search = '';
});
afterAll(() => server.close());

function handlers(paths: PathResponse) {
  server.use(
    http.get(route('/repositories/:id/graph/symbols'), ({ request }) => {
      const q = (new URL(request.url).searchParams.get('q') ?? '').toLowerCase();
      return HttpResponse.json({
        items: SYMBOLS.filter((s) => s.qualified_name.toLowerCase().includes(q)),
        snapshot_id: 's',
        truncated: false,
      });
    }),
    http.get(route('/repositories/:id/graph/path'), ({ request }) => {
      lastPathQuery = new URL(request.url).searchParams;
      return HttpResponse.json(paths);
    }),
  );
}

async function pickFromTo() {
  fireEvent.change(screen.getByLabelText('From symbol'), { target: { value: 'usercontroller' } });
  fireEvent.click(await screen.findByText('UserController.update'));
  fireEvent.change(screen.getByLabelText('To symbol'), { target: { value: 'authorize' } });
  fireEvent.click(await screen.findByText('AuthService.authorize'));
  fireEvent.click(screen.getByRole('button', { name: 'Find path' }));
}

function chainLabels() {
  return within(screen.getByRole('list', { name: 'Impact path' }))
    .getAllByRole('button')
    .map((b) => b.textContent);
}

describe('impact path view', () => {
  it('path_finder_renders_chain', async () => {
    handlers({ paths: [direct], truncated: false });
    renderWithProviders(<PathFinder repoId="repo-1" />);
    await pickFromTo();
    await screen.findByRole('list', { name: 'Impact path' });
    expect(chainLabels()).toEqual([
      'UserController.update',
      'AdminService.updateUser',
      'AuthService.authorize',
    ]);
    expect(screen.getByText('Path minimum confidence 0.55')).toBeTruthy();
    expect(screen.getByText('—CALLS 0.55→')).toBeTruthy();
    expect(lastPathQuery?.get('from')).toBe('k-ctrl');
    expect(lastPathQuery?.get('to')).toBe('k-authorize');
    expect(lastPathQuery?.get('max_depth')).toBe('6');
    expect(lastPathQuery?.get('all')).toBe('true');
    expect(lastPathQuery?.get('max_paths')).toBe('5');
    expect(lastPathQuery?.get('kinds')).toBe('CALLS');
    // A single path has no tabs.
    expect(screen.queryByRole('tablist', { name: 'Alternative paths' })).toBeNull();
  });

  it('no_path_message', async () => {
    handlers({ paths: [], truncated: false });
    renderWithProviders(<PathFinder repoId="repo-1" />);
    fireEvent.change(screen.getByLabelText('Max depth'), { target: { value: '3' } });
    await pickFromTo();
    expect(await screen.findByText('No path within depth 3.')).toBeTruthy();
    expect(lastPathQuery?.get('max_depth')).toBe('3');
  });

  it('alternative_paths_tabs', async () => {
    handlers({ paths: [direct, viaGuard], truncated: false });
    renderWithProviders(<PathFinder repoId="repo-1" />);
    await pickFromTo();
    const tabs = await screen.findByRole('tablist', { name: 'Alternative paths' });
    const [first, second] = within(tabs).getAllByRole('tab');
    expect(first?.textContent).toContain('Path 1');
    expect(first?.getAttribute('aria-selected')).toBe('true');
    expect(chainLabels()).toContain('AdminService.updateUser');
    fireEvent.click(second!);
    expect(second?.getAttribute('aria-selected')).toBe('true');
    expect(chainLabels()).toEqual([
      'UserController.update',
      'AdminGuard.canActivate',
      'AuthService.authorize',
    ]);
    expect(second?.textContent).toContain('min 0.80');
  });

  it('impact_groups_render_categories', async () => {
    const impact: ImpactGraph = {
      root: n('k-authorize', 'AuthService.authorize'),
      categories: [
        {
          name: 'callers',
          items: [{ node: n('k-update', 'AdminService.updateUser'), depth: 1, confidence: 0.55 }],
        },
        {
          name: 'endpoints',
          items: [{ node: n('k-ctrl', 'UserController.update'), depth: 2, confidence: 0.55 }],
        },
        { name: 'tests', items: [] },
      ],
      paths: [direct],
      truncated: false,
    };
    server.use(http.get(route('/reviews/:rid/impact/:key'), () => HttpResponse.json(impact)));
    const open = vi.fn();
    renderWithProviders(
      <ImpactGroups reviewId="run-151" symbolKey="k-authorize" onSymbolClick={open} />,
    );
    const callers = await screen.findByRole('region', { name: 'Impact: callers' });
    expect(within(callers).getByText('callers (1)')).toBeTruthy();
    expect(within(callers).getByText('d1 · 0.55')).toBeTruthy();
    const endpoints = screen.getByRole('region', { name: 'Impact: endpoints' });
    fireEvent.click(within(endpoints).getByText('UserController.update'));
    expect(open).toHaveBeenCalledWith('k-ctrl');
    expect(
      within(screen.getByRole('region', { name: 'Impact: tests' })).getByText('None'),
    ).toBeTruthy();
    expect(screen.getByRole('list', { name: 'Impact path' })).toBeTruthy();
  });

  it('explorer modes switch through the URL', async () => {
    handlers({ paths: [], truncated: false });
    server.use(
      http.get(route('/repositories/:id/intelligence'), () =>
        HttpResponse.json({
          snapshots: [],
          fingerprint: null,
          versions: { tool_version: null, facts_schema_version: null, analyzers: {} },
          stats: null,
          languages: [],
          frameworks: [],
          index_jobs: [],
        }),
      ),
    );
    nav.search = 'mode=path';
    renderWithProviders(<GraphExplorer repoId="repo-1" />);
    expect(screen.getByRole('region', { name: 'Path finder' })).toBeTruthy();
    fireEvent.click(screen.getByRole('tab', { name: 'Impact for review' }));
    expect(nav.replace).toHaveBeenCalledWith('/repositories/repo-1/graph?mode=impact');
  });
});
