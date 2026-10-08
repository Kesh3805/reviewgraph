// @vitest-environment jsdom
import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { GraphExplorer } from '../components/graph/GraphExplorer';
import { SymbolSearch } from '../components/graph/SymbolSearch';
import { DebouncedSearch, SEARCH_DEBOUNCE_MS } from '../lib/debounced-search';
import { explorerSearch, parseExplorerState } from '../lib/explorer-url';
import { AUTHORIZE, AUTHORIZE_DETAIL, intelligenceWithSnapshots } from './graph-fixtures';
import { problem, renderWithProviders, route } from './helpers';

const nav = vi.hoisted(() => ({ search: '', replace: vi.fn(), push: vi.fn() }));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ replace: nav.replace, push: nav.push }),
  usePathname: () => '/repositories/repo-1/graph',
  useSearchParams: () => new URLSearchParams(nav.search),
}));

const server = setupServer();
const requests: URL[] = [];
beforeAll(() => server.listen());
beforeEach(() => {
  nav.search = '';
  requests.length = 0;
  server.use(
    http.get(route('/repositories/:id/graph/symbols'), ({ request }) => {
      requests.push(new URL(request.url));
      return HttpResponse.json({ items: [AUTHORIZE], snapshot_id: 'snap-head', truncated: false });
    }),
    http.get(route('/repositories/:id/graph/symbols/:key'), ({ params }) =>
      params.key === AUTHORIZE.key
        ? HttpResponse.json(AUTHORIZE_DETAIL)
        : problem(404, 'no symbol'),
    ),
    http.get(route('/repositories/:id/intelligence'), () =>
      HttpResponse.json(intelligenceWithSnapshots),
    ),
  );
});
afterEach(() => {
  cleanup();
  server.resetHandlers();
  nav.replace.mockClear();
  vi.useRealTimers();
});
afterAll(() => server.close());

describe('graph explorer search', () => {
  it('search_debounced_and_cancelled', async () => {
    vi.useFakeTimers();
    const signals: { q: string; signal: AbortSignal }[] = [];
    const resolvers: (() => void)[] = [];
    const results: string[] = [];
    const search = new DebouncedSearch<string, string>(
      (q, signal) => {
        signals.push({ q, signal });
        return new Promise((resolve) => resolvers.push(() => resolve(`result:${q}`)));
      },
      (r) => results.push(r),
      () => {},
    );

    search.search('a');
    vi.advanceTimersByTime(100);
    search.search('au');
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS - 1);
    expect(signals).toHaveLength(0);
    vi.advanceTimersByTime(1);
    // Only the last keystroke fires a request.
    expect(signals.map((s) => s.q)).toEqual(['au']);

    // A new query aborts the in-flight one; its late result is dropped.
    search.search('aut');
    expect(signals[0]?.signal.aborted).toBe(true);
    vi.advanceTimersByTime(SEARCH_DEBOUNCE_MS);
    expect(signals.map((s) => s.q)).toEqual(['au', 'aut']);
    resolvers[0]?.();
    resolvers[1]?.();
    await vi.runAllTimersAsync();
    expect(results).toEqual(['result:aut']);
  });

  it('kind_filter_applied', async () => {
    renderWithProviders(<SymbolSearch repoId="repo-1" onSelect={() => {}} />);
    fireEvent.change(screen.getByLabelText('Search symbols'), { target: { value: 'authorize' } });
    expect(await screen.findByText('AuthService.authorize')).toBeTruthy();
    expect(requests.at(-1)?.searchParams.get('kind')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'method' }));
    fireEvent.click(screen.getByRole('button', { name: 'endpoint' }));
    await waitFor(() => expect(requests.at(-1)?.searchParams.get('kind')).toBe('method,endpoint'));
    expect(requests.at(-1)?.searchParams.get('q')).toBe('authorize');
    expect(screen.getByText('in 4 · out 2')).toBeTruthy();
    expect(screen.getByText('src/auth/auth.service.ts:10')).toBeTruthy();
  });

  it('deep_link_opens_symbol', async () => {
    nav.search = 'symbol=k-authorize&snapshot=snap-head';
    renderWithProviders(<GraphExplorer repoId="repo-1" />);
    const panel = await screen.findByRole('region', { name: 'Symbol' });
    expect(panel.textContent).toContain('AuthService.authorize');
    expect(panel.textContent).toContain('authorize(user: User, action: Action): Promise<void>');
    expect(panel.textContent).toContain('nest.injectable');
    // The deep-linked PR head snapshot is offered even though it is not in the list.
    expect((screen.getByLabelText('Snapshot') as HTMLSelectElement).value).toBe('snap-head');

    expect(parseExplorerState(new URLSearchParams('symbol=a%3Ab&mode=bogus'))).toEqual({
      mode: 'search',
      symbol: 'a:b',
      snapshot: undefined,
      review: undefined,
    });
    expect(explorerSearch({ mode: 'search', symbol: 'a:b', snapshot: 's' })).toBe(
      '?symbol=a%3Ab&snapshot=s',
    );
  });

  it('selecting a result updates the URL', async () => {
    renderWithProviders(<GraphExplorer repoId="repo-1" />);
    fireEvent.change(screen.getByLabelText('Search symbols'), { target: { value: 'auth' } });
    fireEvent.click(await screen.findByText('AuthService.authorize'));
    expect(nav.replace).toHaveBeenCalledWith('/repositories/repo-1/graph?symbol=k-authorize');
  });

  it('snapshot_select_changes_query', async () => {
    renderWithProviders(<GraphExplorer repoId="repo-1" />);
    const select = (await screen.findByLabelText('Snapshot')) as HTMLSelectElement;
    await screen.findByRole('option', { name: /full · main/ });
    fireEvent.change(select, { target: { value: 'snap-main' } });
    expect(nav.replace).toHaveBeenCalledWith('/repositories/repo-1/graph?snapshot=snap-main');
    cleanup();

    nav.search = 'snapshot=snap-main';
    renderWithProviders(<GraphExplorer repoId="repo-1" />);
    fireEvent.change(screen.getByLabelText('Search symbols'), { target: { value: 'auth' } });
    await screen.findByText('AuthService.authorize');
    expect(requests.at(-1)?.searchParams.get('snapshot')).toBe('snap-main');
  });

  it('engine unavailable shows a retry', async () => {
    server.use(
      http.get(route('/repositories/:id/graph/symbols'), () => problem(503, 'engine down')),
    );
    renderWithProviders(<SymbolSearch repoId="repo-1" onSelect={() => {}} />);
    fireEvent.change(screen.getByLabelText('Search symbols'), { target: { value: 'x' } });
    expect(await screen.findByText('Graph service unavailable.')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeTruthy();
  });
});
