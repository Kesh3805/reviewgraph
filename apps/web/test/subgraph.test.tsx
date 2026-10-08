// @vitest-environment jsdom
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { delay, http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { graphStylesheet } from '../components/graph/styles';
import { SubgraphView } from '../components/graph/SubgraphView';
import type { GraphEdge, GraphNode, Subgraph, SubgraphRequest } from '../lib/api/pending';
import { MAX_SUBGRAPH_NODES, mergeSubgraph, toElements } from '../lib/subgraph';
import { renderWithProviders, route } from './helpers';

vi.mock('../components/graph/CytoscapeGraph', () => import('./cytoscape-stub'));

const node = (key: string, name = key, kind = 'method'): GraphNode => ({
  key,
  name,
  qualified_name: name,
  kind,
  path: null,
  line: null,
});
const edge = (source: string, target: string, confidence = 0.9): GraphEdge => ({
  source,
  target,
  kind: 'CALLS',
  confidence,
});

const SEED = 'k-authorize';
const initial: Subgraph = {
  nodes: [
    node(SEED, 'AuthService.authorize'),
    node('k-update', 'AdminService.updateUser'),
    node('k-ctrl', 'UserController.update', 'endpoint'),
  ],
  edges: [edge('k-update', SEED, 0.95), edge('k-ctrl', 'k-update', 0.5)],
  truncated: false,
};

const server = setupServer();
const bodies: SubgraphRequest[] = [];
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
  bodies.length = 0;
});
afterAll(() => server.close());

function subgraphHandler(respond: (body: SubgraphRequest) => Subgraph | Promise<Subgraph>) {
  server.use(
    http.post(route('/repositories/:id/graph/subgraph'), async ({ request }) => {
      const body = (await request.json()) as SubgraphRequest;
      bodies.push(body);
      return HttpResponse.json(await respond(body));
    }),
  );
}

function nodeLabels() {
  return within(screen.getByRole('list', { name: 'Graph nodes' }))
    .getAllByRole('button')
    .map((b) => b.textContent);
}

describe('subgraph view', () => {
  it('renders_subgraph_nodes_edges', async () => {
    subgraphHandler(() => initial);
    renderWithProviders(<SubgraphView repoId="repo-1" seed={SEED} />);
    await screen.findByTestId('cytoscape');
    expect(nodeLabels()).toEqual([
      'AuthService.authorize',
      'AdminService.updateUser',
      'UserController.update',
    ]);
    expect(
      within(screen.getByRole('list', { name: 'Graph edges' })).getAllByRole('listitem'),
    ).toHaveLength(2);
    expect(bodies[0]).toEqual({
      seeds: [SEED],
      depth: 1,
      kinds: ['callers', 'callees'],
      max_nodes: MAX_SUBGRAPH_NODES,
    });
    expect(screen.getByText('3 / 500 nodes')).toBeTruthy();
  });

  it('expand_merges_without_duplicates', async () => {
    subgraphHandler((body) =>
      body.seeds[0] === SEED
        ? initial
        : {
            // The expansion repeats known nodes and edges and adds one new caller.
            nodes: [
              node('k-ctrl', 'UserController.update', 'endpoint'),
              node('k-update', 'AdminService.updateUser'),
              node('k-test', 'admin.spec', 'test'),
            ],
            edges: [edge('k-ctrl', 'k-update', 0.5), edge('k-test', 'k-ctrl', 0.8)],
            truncated: false,
          },
    );
    renderWithProviders(<SubgraphView repoId="repo-1" seed={SEED} />);
    await screen.findByTestId('cytoscape');
    fireEvent.doubleClick(screen.getByRole('button', { name: 'UserController.update' }));
    await waitFor(() => expect(nodeLabels()).toHaveLength(4));
    expect(nodeLabels()).toEqual([
      'AuthService.authorize',
      'AdminService.updateUser',
      'UserController.update',
      'admin.spec',
    ]);
    expect(
      within(screen.getByRole('list', { name: 'Graph edges' })).getAllByRole('listitem'),
    ).toHaveLength(3);
    expect(bodies[1]).toMatchObject({ seeds: ['k-ctrl'], depth: 1 });

    const merged = mergeSubgraph(
      { nodes: initial.nodes, edges: initial.edges },
      { nodes: initial.nodes, edges: initial.edges },
    );
    expect(merged).toMatchObject({ status: 'merged', added: 0 });
    expect(merged.graph.edges).toHaveLength(2);
  });

  it('expansion_capped_at_500', async () => {
    const many = (prefix: string, n: number) =>
      Array.from({ length: n }, (_, i) => node(`${prefix}${i}`));
    expect(
      mergeSubgraph({ nodes: many('a', 450), edges: [] }, { nodes: many('b', 51), edges: [] }),
    ).toMatchObject({
      status: 'refused',
      wouldHave: 501,
    });
    expect(
      mergeSubgraph({ nodes: many('a', 450), edges: [] }, { nodes: many('b', 50), edges: [] }),
    ).toMatchObject({
      status: 'merged',
      added: 50,
    });

    subgraphHandler((body) =>
      body.seeds[0] === SEED
        ? { nodes: [node(SEED), ...many('a', 449)], edges: [], truncated: false }
        : { nodes: many('b', 60), edges: [], truncated: false },
    );
    renderWithProviders(<SubgraphView repoId="repo-1" seed={SEED} />);
    await screen.findByTestId('cytoscape');
    fireEvent.doubleClick(screen.getByRole('button', { name: 'a0' }));
    expect((await screen.findByTestId('banner-refused')).textContent).toContain('limit 500');
    expect(nodeLabels()).toHaveLength(450);
  });

  it('low_confidence_edges_dashed', () => {
    const elements = toElements({ nodes: initial.nodes, edges: initial.edges }, [SEED]);
    const edges = elements.filter((e) => e.data.source);
    expect(edges.map((e) => [e.data.confidence, e.data.low])).toEqual([
      [0.95, false],
      [0.5, true],
    ]);
    const dashed = (
      graphStylesheet() as { selector: string; style: Record<string, unknown> }[]
    ).find((s) => s.selector === 'edge[?low]');
    expect(dashed?.style['line-style']).toBe('dashed');
    const edgeStyle = (
      graphStylesheet() as { selector: string; style: Record<string, unknown> }[]
    ).find((s) => s.selector === 'edge');
    expect(edgeStyle?.style.width).toBe('mapData(confidence, 0, 1, 1, 5)');
  });

  it('kind_toggles_refetch', async () => {
    subgraphHandler(() => initial);
    renderWithProviders(<SubgraphView repoId="repo-1" seed={SEED} />);
    await screen.findByTestId('cytoscape');
    fireEvent.click(screen.getByRole('button', { name: 'Tests' }));
    await waitFor(() => expect(bodies).toHaveLength(2));
    expect(bodies[1]?.kinds).toEqual(['callers', 'callees', 'tests']);
    fireEvent.change(screen.getByLabelText('Depth'), { target: { value: '2' } });
    await waitFor(() => expect(bodies).toHaveLength(3));
    expect(bodies[2]).toMatchObject({ depth: 2, kinds: ['callers', 'callees', 'tests'] });
  });

  it('truncated result and a single expansion in flight', async () => {
    let expansions = 0;
    subgraphHandler(async (body) => {
      if (body.seeds[0] === SEED) return { ...initial, truncated: true };
      expansions += 1;
      await delay(50);
      return { nodes: [], edges: [], truncated: false };
    });
    renderWithProviders(<SubgraphView repoId="repo-1" seed={SEED} />);
    expect(await screen.findByTestId('banner-truncated')).toBeTruthy();
    fireEvent.doubleClick(screen.getByRole('button', { name: 'UserController.update' }));
    await screen.findByText('Expanding…');
    fireEvent.doubleClick(screen.getByRole('button', { name: 'AdminService.updateUser' }));
    await waitFor(() => expect(screen.queryByText('Expanding…')).toBeNull());
    expect(expansions).toBe(1);
  });
});
