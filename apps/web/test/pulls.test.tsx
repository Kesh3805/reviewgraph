// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { PullsView } from '../components/pulls/PullsView';
import { RunStateBadge } from '../components/pulls/RunStateBadge';
import type { Page, PullRequestSummary, ReviewState } from '../lib/api/pending';
import { parsePullFilters, pullFiltersToSearch } from '../lib/pull-filters';
import { RUN_POLL_MS, pullsRefetchInterval } from '../lib/queries';
import { REVIEW_STATES } from '../lib/run-state';
import { ORG_ID, renderWithProviders, route } from './helpers';

const nav = vi.hoisted(() => ({
  search: '',
  replace: vi.fn(),
  push: vi.fn(),
}));
vi.mock('next/navigation', () => ({
  useRouter: () => ({ replace: nav.replace, push: nav.push }),
  usePathname: () => '/pull-requests',
  useSearchParams: () => new URLSearchParams(nav.search),
}));

function pr(overrides: Partial<PullRequestSummary> = {}): PullRequestSummary {
  return {
    id: 'pr-1',
    repository_id: 'repo-1',
    repository_full_name: 'acme/api',
    number: 151,
    title: 'Allow admins to update users',
    author: 'octocat',
    state: 'open',
    draft: false,
    head_sha: '0123456789abcdef',
    url: null,
    latest_run: {
      id: 'run-9',
      state: 'COMPLETED',
      degraded: true,
      stage: null,
      created_at: '2026-10-07T10:00:00Z',
    },
    findings_by_severity: { critical: 1, high: 2, medium: 0, low: 0, info: 0 },
    updated_at: '2026-10-07T10:05:00Z',
    ...overrides,
  };
}

const server = setupServer();
let lastQuery: URLSearchParams | null = null;
beforeAll(() => server.listen());
beforeEach(() => {
  nav.search = '';
  lastQuery = null;
  server.use(
    http.get(route('/repositories'), () => HttpResponse.json({ items: [], next_cursor: null })),
  );
});
afterEach(() => {
  cleanup();
  server.resetHandlers();
  nav.replace.mockClear();
  nav.push.mockClear();
});
afterAll(() => server.close());

function pullsHandler(page: Page<PullRequestSummary>) {
  server.use(
    http.get(route('/pull-requests'), ({ request }) => {
      lastQuery = new URL(request.url).searchParams;
      return HttpResponse.json(page);
    }),
  );
}

describe('pull requests', () => {
  it('pulls_table_filters_via_url', async () => {
    nav.search = 'state=open&severity=high&has_findings=true';
    pullsHandler({ items: [pr()], next_cursor: null });
    renderWithProviders(<PullsView />);

    expect(await screen.findByText('#151 Allow admins to update users')).toBeTruthy();
    expect(lastQuery?.get('organization_id')).toBe(ORG_ID);
    expect(lastQuery?.get('state')).toBe('open');
    expect(lastQuery?.get('severity')).toBe('high');
    expect(lastQuery?.get('has_findings')).toBe('true');
    expect((screen.getByLabelText('Severity') as HTMLSelectElement).value).toBe('high');

    fireEvent.change(screen.getByLabelText('Severity'), { target: { value: 'critical' } });
    expect(nav.replace).toHaveBeenCalledWith(
      '/pull-requests?state=open&has_findings=true&severity=critical',
    );

    // Round trip and junk values are ignored.
    const filters = parsePullFilters(new URLSearchParams('state=bogus&severity=high&cursor=c1'));
    expect(filters).toEqual({ severity: 'high', cursor: 'c1' });
    expect(pullFiltersToSearch(filters)).toBe('?severity=high&cursor=c1');
  });

  it('run_state_badge_all_states', () => {
    const labels = new Map<string, string>();
    for (const state of REVIEW_STATES) {
      const { container, unmount } = render(<RunStateBadge state={state} />);
      labels.set(state, container.textContent ?? '');
      unmount();
    }
    expect(Object.fromEntries(labels)).toEqual({
      RECEIVED: 'Queued',
      INDEXING: 'Indexing',
      ANALYZING: 'Analyzing',
      REVIEWING: 'Reviewing',
      VERIFYING: 'Verifying',
      PUBLISHING: 'Publishing',
      COMPLETED: 'Completed',
      FAILED_INDEXING: 'Failed: indexing',
      FAILED_ANALYSIS: 'Failed: analysis',
      FAILED_REVIEW: 'Failed: review',
      FAILED_PUBLISH: 'Failed: publish',
      SUPERSEDED: 'Superseded',
      CANCELLED: 'Cancelled',
    } satisfies Record<ReviewState, string>);

    const { container } = render(<RunStateBadge state="COMPLETED" degraded />);
    expect(container.textContent).toBe('Completed (degraded)');
    expect(container.querySelector('[data-state]')?.className).not.toContain('emerald');
  });

  it('manual_review_maintainer_only', async () => {
    pullsHandler({ items: [pr()], next_cursor: null });
    let triggered = '';
    server.use(
      http.post(route('/pull-requests/:id/review'), ({ params }) => {
        triggered = String(params.id);
        return HttpResponse.json({ review_id: 'run-10', created: true });
      }),
    );

    const viewer = renderWithProviders(<PullsView />, { role: 'viewer' });
    await screen.findByText('#151 Allow admins to update users');
    expect(screen.queryByRole('button', { name: /review #151 now/i })).toBeNull();
    viewer.unmount();

    renderWithProviders(<PullsView />, { role: 'member' });
    fireEvent.click(await screen.findByRole('button', { name: 'Review #151 now' }));
    expect(await screen.findByText('Review requested.')).toBeTruthy();
    expect(triggered).toBe('pr-1');
    // Never a retry button.
    expect(screen.queryByRole('button', { name: /retry/i })).toBeNull();
  });

  it('pagination_cursor', async () => {
    nav.search = 'state=open';
    pullsHandler({ items: [pr()], next_cursor: 'cursor-2' });
    const first = renderWithProviders(<PullsView />);
    fireEvent.click(await screen.findByRole('button', { name: 'Next page' }));
    expect(nav.push).toHaveBeenCalledWith('/pull-requests?state=open&cursor=cursor-2');
    first.unmount();

    nav.search = 'state=open&cursor=cursor-2';
    pullsHandler({ items: [pr({ id: 'pr-2', number: 152 })], next_cursor: null });
    renderWithProviders(<PullsView />);
    await screen.findByText(/#152/);
    expect(lastQuery?.get('cursor')).toBe('cursor-2');
    expect(screen.queryByRole('button', { name: 'Next page' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'First page' }));
    expect(nav.push).toHaveBeenLastCalledWith('/pull-requests?state=open');
  });

  it('degraded and superseded runs show their badges; rows poll while running', async () => {
    pullsHandler({
      items: [
        pr(),
        pr({
          id: 'pr-3',
          number: 153,
          latest_run: { ...pr().latest_run!, id: 'r3', state: 'SUPERSEDED', degraded: false },
        }),
      ],
      next_cursor: null,
    });
    renderWithProviders(<PullsView />);
    const table = await screen.findByRole('table');
    await waitFor(() => expect(within(table).getByText('Completed (degraded)')).toBeTruthy());
    expect(within(table).getByText('Superseded')).toBeTruthy();
    expect(within(table).getByText('#151 Allow admins to update users').getAttribute('href')).toBe(
      '/reviews/run-9',
    );

    expect(pullsRefetchInterval({ items: [pr()], next_cursor: null })).toBe(false);
    const running = pr({ latest_run: { ...pr().latest_run!, state: 'REVIEWING' } });
    expect(pullsRefetchInterval({ items: [running], next_cursor: null })).toBe(RUN_POLL_MS);
  });
});
