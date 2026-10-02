// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import type { ReactNode } from 'react';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { Dashboard } from '../components/dashboard/Dashboard';
import {
  activeRunsRefetchInterval,
  ACTIVE_RUNS_POLL_MS,
  type DashboardSummary,
} from '../lib/dashboard';

const summary: DashboardSummary = {
  window_days: 7,
  reviews: { completed: 12, degraded: 2, failed: 1 },
  latency_ms: { median: 41_000, p95: 120_000 },
  findings_by_severity: { critical: 1, high: 4, medium: 9, low: 3, info: 0 },
  quality: { acceptance_rate: 0.62, false_positive_rate: 0.08 },
  active_runs: [
    {
      id: 'run-1',
      repository: 'acme/api',
      pull_request_number: 42,
      state: 'REVIEWING',
      stage: 'reviewer_execution',
      started_at: new Date(Date.now() - 90_000).toISOString(),
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

function wrap(ui: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

const dashboardUrl = '*/api/v1/organizations/:id/dashboard';

describe('Dashboard', () => {
  it('dashboard_renders_cards', async () => {
    server.use(http.get(dashboardUrl, () => HttpResponse.json(summary)));
    wrap(<Dashboard orgId="org-1" />);

    await screen.findByText('acme/api#42');
    const reviews = screen.getByTestId('card-Reviews');
    expect(within(reviews).getByText('12')).toBeTruthy();
    expect(within(screen.getByTestId('card-Review latency')).getByText('41.0s')).toBeTruthy();
    expect(within(screen.getByTestId('card-Review latency')).getByText('120.0s')).toBeTruthy();
    expect(
      within(screen.getByTestId('card-Published findings')).getByText('Critical'),
    ).toBeTruthy();
    expect(within(screen.getByTestId('card-Finding quality')).getByText('62.0%')).toBeTruthy();
    expect(screen.getByText('acme/api#42')).toBeTruthy();
    expect(screen.getByText('REVIEWING')).toBeTruthy();

    // Every metric links to its source list.
    for (const card of ['Reviews', 'Review latency', 'Published findings', 'Finding quality']) {
      const link = within(screen.getByTestId(`card-${card}`)).getByRole('link');
      expect(link.getAttribute('href')).toMatch(/^\/pull-requests/);
    }
  });

  it('passes the organization id from the session, not user input', async () => {
    let requested = '';
    server.use(
      http.get(dashboardUrl, ({ params }) => {
        requested = String(params.id);
        return HttpResponse.json(summary);
      }),
    );
    wrap(<Dashboard orgId="org-42" />);
    await screen.findByText('acme/api#42');
    expect(requested).toBe('org-42');
  });

  it('one failed metric does not blank the page', async () => {
    server.use(http.get(dashboardUrl, () => HttpResponse.json({ ...summary, reviews: null })));
    wrap(<Dashboard orgId="org-1" />);
    expect(
      await within(await screen.findByTestId('card-Reviews')).findByText('Not available yet.'),
    ).toBeTruthy();
    expect(within(screen.getByTestId('card-Finding quality')).getByText('62.0%')).toBeTruthy();
  });

  it('shows card-level error states when the request fails', async () => {
    server.use(
      http.get(dashboardUrl, () =>
        HttpResponse.json(
          { title: 'internal server error', status: 500 },
          { status: 500, headers: { 'content-type': 'application/problem+json' } },
        ),
      ),
    );
    wrap(<Dashboard orgId="org-1" />);
    const alerts = await screen.findAllByRole('alert');
    expect(alerts.length).toBeGreaterThanOrEqual(4);
    // The page itself (heading, cards) is still rendered.
    expect(screen.getByRole('heading', { name: 'Dashboard' })).toBeTruthy();
  });

  it('active_runs_polls_only_when_nonterminal', () => {
    expect(activeRunsRefetchInterval(undefined)).toBe(false);
    expect(activeRunsRefetchInterval(summary)).toBe(ACTIVE_RUNS_POLL_MS);
    expect(ACTIVE_RUNS_POLL_MS).toBe(5000);
    const settled = {
      ...summary,
      active_runs: summary.active_runs.map((r) => ({ ...r, state: 'COMPLETED' as const })),
    };
    expect(activeRunsRefetchInterval(settled)).toBe(false);
    expect(activeRunsRefetchInterval({ ...summary, active_runs: [] })).toBe(false);
  });
});
