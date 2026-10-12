// @vitest-environment jsdom
import './dom-stubs';
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { ImpactGroups } from '../components/graph/ImpactGroups';
import { PathFinder } from '../components/graph/PathFinder';
import type {
  GraphEdge,
  GraphNode,
  GraphPath,
  ImpactResponse,
  PathResponse,
  SymbolSearchResult,
} from '../lib/api/pending';
import { renderWithProviders, route } from './helpers';

const node = (key: string, name: string): GraphNode => ({
  key,
  name,
  qualified_name: name,
  kind: 'method',
  path: null,
  line: null,
});
const edge = (source: string, target: string, confidence = 0.9): GraphEdge => ({
  source,
  target,
  kind: 'CALLS',
  confidence,
});
const sym = (key: string, name: string): SymbolSearchResult => ({
  key,
  name,
  qualified_name: name,
  kind: 'method',
  path: 'a.ts',
  line: 1,
  in_degree: 0,
  out_degree: 0,
});

const chain: GraphPath = {
  nodes: [
    node('k-ctrl', 'UserController.update'),
    node('k-svc', 'AdminService.updateUser'),
    node('k-auth', 'AuthService.authorize'),
  ],
  edges: [edge('k-ctrl', 'k-svc', 0.95), edge('k-svc', 'k-auth', 0.5)],
};
const alt: GraphPath = {
  nodes: [node('k-ctrl', 'UserController.update'), node('k-auth', 'AuthService.authorize')],
  edges: [edge('k-ctrl', 'k-auth', 0.7)],
};

const server = setupServer();
let pathQueries: URLSearchParams[] = [];
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
  pathQueries = [];
});
afterAll(() => server.close());

function serve(response: PathResponse) {
  server.use(
    http.get(route('/repositories/:id/graph/symbols'), ({ request }) => {
      const q = new URL(request.url).searchParams.get('q');
      const items =
        q === 'update'
          ? [sym('k-ctrl', 'UserController.update')]
          : [sym('k-auth', 'AuthService.authorize')];
      return HttpResponse.json({ items, snapshot_id: 's1', truncated: false });
    }),
    http.get(route('/repositories/:id/graph/path'), ({ request }) => {
      pathQueries.push(new URL(request.url).searchParams);
      return HttpResponse.json(response);
    }),
  );
}

async function pick(label: string, text: string, name: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value: text } });
  fireEvent.click(await screen.findByRole('button', { name: new RegExp(name) }));
}

async function findPath() {
  await pick('From symbol', 'update', 'UserController.update');
  await pick('To symbol', 'authorize', 'AuthService.authorize');
  fireEvent.click(screen.getByRole('button', { name: 'Find path' }));
}

describe('path finder', () => {
  it('path_finder_renders_chain', async () => {
    serve({ paths: [chain] });
    renderWithProviders(<PathFinder repoId="repo-1" />);
    await findPath();
    const list = await screen.findByRole('list', { name: 'Impact path' });
    expect(
      within(list)
        .getAllByRole('button')
        .map((b) => b.textContent),
    ).toEqual(['UserController.update', 'AdminService.updateUser', 'AuthService.authorize']);
    expect(screen.getByText('Path minimum confidence 0.50')).toBeTruthy();
    expect(pathQueries[0]?.get('all')).toBe('true');
    expect(pathQueries[0]?.get('max_paths')).toBe('5');
    expect(pathQueries[0]?.get('from')).toBe('k-ctrl');
    expect(pathQueries[0]?.get('to')).toBe('k-auth');
  });

  it('no_path_message', async () => {
    serve({ paths: [] });
    renderWithProviders(<PathFinder repoId="repo-1" />);
    await findPath();
    expect(await screen.findByText('No path within depth 4.')).toBeTruthy();
  });

  it('alternative_paths_tabs', async () => {
    serve({ paths: [chain, alt] });
    renderWithProviders(<PathFinder repoId="repo-1" />);
    await findPath();
    const tabs = await screen.findAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual(['Path 1 (2 hops)', 'Path 2 (1 hops)']);
    fireEvent.click(tabs[1] as HTMLElement);
    await waitFor(() =>
      expect(
        within(screen.getByRole('list', { name: 'Impact path' })).getAllByRole('button'),
      ).toHaveLength(2),
    );
  });
});

describe('impact groups', () => {
  it('impact_groups_render_categories', () => {
    const impact: ImpactResponse = {
      symbol_key: 'k-auth',
      path: chain,
      truncated: false,
      categories: {
        callers: [
          {
            key: 'k-svc',
            name: 'updateUser',
            qualified_name: 'AdminService.updateUser',
            kind: 'method',
            path: null,
            distance: 1,
            min_confidence: 0.9,
          },
        ],
        tests: [
          {
            key: 'k-t',
            name: 'auth.spec',
            kind: 'test',
            path: null,
            distance: 2,
            min_confidence: 0.6,
          },
        ],
      },
    };
    renderWithProviders(<ImpactGroups impact={impact} />);
    expect(screen.getByRole('region', { name: 'Callers' })).toBeTruthy();
    expect(screen.getByRole('region', { name: 'Tests' })).toBeTruthy();
    expect(screen.queryByRole('region', { name: 'Callees' })).toBeNull();
    expect(screen.getByText('distance 1 · confidence 0.90')).toBeTruthy();
    expect(screen.getByRole('list', { name: 'Impact path' })).toBeTruthy();
  });
});
