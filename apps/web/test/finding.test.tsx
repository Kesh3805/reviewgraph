// @vitest-environment jsdom
import './dom-stubs';
import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { computeConfidence } from '../components/finding/ConfidenceBreakdown';
import { FindingDetailView } from '../components/finding/FindingDetailView';
import { buildFlow } from '../lib/impact-flow';
import { FINDING_ID, excerpt, findingDetail, findingTrace } from './finding-fixtures';
import { renderWithProviders, route } from './helpers';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));
// The shiki server action is not available in tests; excerpts stay plain text.
vi.mock('../lib/highlight', () => ({ highlight: vi.fn(async () => null) }));

const BASE_TEXT =
  'async updateUser(id, dto) {\n  await this.permissions.check(user);\n  return save(dto);\n}';
const HEAD_TEXT = 'async updateUser(id, dto) {\n  return save(dto);\n}';

const server = setupServer();
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
  push.mockClear();
});
afterAll(() => server.close());

function handlers({ detail = findingDetail(), trace = findingTrace(), baseStatus = 200 } = {}) {
  server.use(
    http.get(route('/findings/:id'), () => HttpResponse.json(detail)),
    http.get(route('/findings/:id/trace'), () => HttpResponse.json(trace)),
    http.get(route('/repositories/:id/source'), ({ request }) => {
      const snapshot = new URL(request.url).searchParams.get('snapshot');
      if (snapshot === 'snap-base') {
        if (baseStatus === 404) return HttpResponse.json({ status: 404 }, { status: 404 });
        return HttpResponse.json(excerpt('snap-base', BASE_TEXT));
      }
      return HttpResponse.json(excerpt('snap-head', HEAD_TEXT));
    }),
  );
}

describe('finding detail', () => {
  it('impact_path_renders_nodes_in_order', async () => {
    handlers();
    renderWithProviders(<FindingDetailView findingId={FINDING_ID} />);
    const chain = await screen.findByRole('list', { name: 'Impact path' });
    const labels = within(chain)
      .getAllByRole('button')
      .map((b) => b.textContent);
    expect(labels).toEqual([
      'UserController.update',
      'AdminService.updateUser',
      'AuthService.authorize',
    ]);

    // Left to right: x grows from the entrypoint to the changed symbol.
    const flow = buildFlow(findingTrace().impact_path!);
    const xs = flow.nodes.map((n) => n.position.x);
    expect([...xs].sort((a, b) => a - b)).toEqual(xs);
    expect(flow.nodes.map((n) => n.data.role)).toEqual(['entrypoint', 'hop', 'changed']);
    expect(flow.edges.map((e) => e.label)).toEqual(['CALLS 0.95', 'CALLS 0.55']);

    // Clicking a node opens the GX symbol page.
    fireEvent.click(within(chain).getByRole('button', { name: 'AuthService.authorize' }));
    expect(push).toHaveBeenCalledWith(
      '/repositories/repo-1/graph?symbol=k-authorize&snapshot=snap-head',
    );
  });

  it('low_confidence_edge_dashed', () => {
    const flow = buildFlow(findingTrace().impact_path!);
    const [high, low] = flow.edges;
    expect(high?.style?.strokeDasharray).toBeUndefined();
    expect(low?.style?.strokeDasharray).toBe('6 4');
    expect(low?.data?.dashed).toBe(true);
  });

  it('base_head_added_file_message', async () => {
    const trace = findingTrace();
    handlers({ trace: { ...trace, base_head: { ...trace.base_head!, base: null } } });
    renderWithProviders(<FindingDetailView findingId={FINDING_ID} />);
    const base = await screen.findByTestId('side-base');
    expect(within(base).getByText('The file did not exist at base.')).toBeTruthy();
    const head = screen.getByTestId('side-head');
    expect(await within(head).findByText('return save(dto);', { exact: false })).toBeTruthy();
    expect(within(head).getByText('calls PermissionService.check: does not hold')).toBeTruthy();
    cleanup();

    // A 404 for the base excerpt means the same thing.
    handlers({ baseStatus: 404 });
    renderWithProviders(<FindingDetailView findingId={FINDING_ID} />);
    expect(
      await within(await screen.findByTestId('side-base')).findByText(
        'The file did not exist at base.',
      ),
    ).toBeTruthy();
  });

  it('shows base and head excerpts with the predicate result', async () => {
    handlers();
    renderWithProviders(<FindingDetailView findingId={FINDING_ID} />);
    const base = await screen.findByTestId('side-base');
    expect(
      await within(base).findByText('await this.permissions.check(user);', { exact: false }),
    ).toBeTruthy();
    expect(within(base).getByText('calls PermissionService.check: holds')).toBeTruthy();
  });

  it('confidence_components_sum_matches', async () => {
    handlers();
    renderWithProviders(<FindingDetailView findingId={FINDING_ID} />);
    const sum = await screen.findByTestId('confidence-sum');
    expect(sum.textContent).toBe('0.77');
    expect(computeConfidence(findingDetail().confidence_components)).toBeCloseTo(0.77, 5);
    expect(screen.queryByText('The components do not add up to the stored confidence.')).toBeNull();
    const terms = within(screen.getByRole('list', { name: 'Confidence components' }))
      .getAllByRole('listitem')
      .map((li) => li.firstChild?.textContent);
    expect(terms).toEqual([
      'anchor',
      'deterministic',
      'graph',
      'repo',
      'reproduction',
      'agreement',
      'contradiction',
      'uncertainty',
    ]);
  });

  it('no_prompt_text_rendered', async () => {
    const PROMPT = 'SYSTEM PROMPT: you are a security reviewer';
    const RAW = 'RAW MODEL OUTPUT {"findings": []}';
    // Even if the API leaked extra fields, the page renders typed fields only.
    const detail = { ...findingDetail(), prompt: PROMPT, raw_output: RAW };
    const trace = { ...findingTrace(), prompt: PROMPT, model_reasoning: RAW };
    handlers({ detail, trace });
    const { container } = renderWithProviders(<FindingDetailView findingId={FINDING_ID} />);
    await screen.findByRole('list', { name: 'Impact path' });
    await screen.findAllByTestId('source-excerpt');
    expect(container.textContent).not.toContain('SYSTEM PROMPT');
    expect(container.textContent).not.toContain('RAW MODEL OUTPUT');
  });

  it('incomplete trace shows a notice and the available stages', async () => {
    handlers({ trace: findingTrace({ incomplete: true, impact_path: null, base_head: null }) });
    renderWithProviders(<FindingDetailView findingId={FINDING_ID} />);
    expect(await screen.findByText(/This trace is incomplete/)).toBeTruthy();
    expect(screen.getByText('No impact path recorded.')).toBeTruthy();
    expect(screen.getByText('NOT EXECUTED')).toBeTruthy();
    expect(screen.getByTestId('policy-GuardOnEndpoint').textContent).toContain(
      'explicit rule-guards',
    );
  });
});
