// @vitest-environment jsdom
import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { ReviewDetailView } from '../components/review/ReviewDetailView';
import { TraceLink, traceUrl } from '../components/review/TraceLink';
import { groupFindings } from '../lib/findings';
import { REVIEW_ID, reviewDetail, reviewFindings } from './fixtures';
import { renderWithProviders, route } from './helpers';

const push = vi.fn();
vi.mock('next/navigation', () => ({ useRouter: () => ({ push }) }));

const server = setupServer();
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
  vi.unstubAllEnvs();
});
afterAll(() => server.close());

function handlers(detail = reviewDetail()) {
  server.use(
    http.get(route('/reviews/:id'), () => HttpResponse.json(detail)),
    http.get(route('/reviews/:id/findings'), () => HttpResponse.json({ items: reviewFindings })),
    http.get(route('/pull-requests/:id/reviews'), () =>
      HttpResponse.json({
        items: [
          { ...detail, head_sha: 'bbbbbbb1111111', superseded_by: null },
          {
            ...detail,
            id: 'run-150',
            state: 'SUPERSEDED',
            degraded: false,
            head_sha: 'ccccccc2222222',
            superseded_by: REVIEW_ID,
          },
        ],
      }),
    ),
  );
}

describe('review detail', () => {
  it('completeness_shows_failed_reviewer', async () => {
    handlers();
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />);
    const failed = await screen.findByTestId('reviewer-performance');
    expect(within(failed).getByText('Failed — timed out')).toBeTruthy();
    expect(within(screen.getByTestId('reviewer-security')).getByText('Succeeded')).toBeTruthy();
    expect(screen.getByText(/1 of 2 planned reviewers succeeded, 1 failed/)).toBeTruthy();
    expect(screen.getByText('Completed (degraded)')).toBeTruthy();
    expect(screen.getByText('auth_path')).toBeTruthy();
  });

  it('not_executed_never_green', async () => {
    handlers();
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />);
    const chip = await screen.findByTestId('not-executed');
    expect(chip.textContent).toBe('Not executed — reproduction: no test runner configured');
    expect(chip.className).not.toMatch(/emerald|green/);
    expect(chip.className).toContain('bg-muted');
  });

  it('suppressed_grouped_by_reason', async () => {
    handlers();
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />);
    const toggle = await screen.findByRole('button', { name: 'Suppressed (2)' });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByText('Possible N+1 query')).toBeNull();
    fireEvent.click(toggle);
    const low = screen.getByTestId('suppressed-SUPPRESSED_LOW_CONFIDENCE');
    expect(within(low).getByText('Low confidence (1)')).toBeTruthy();
    expect(within(low).getByText('Possible N+1 query')).toBeTruthy();
    const dup = screen.getByTestId('suppressed-SUPPRESSED_DUPLICATE');
    expect(within(dup).getByText('Same as f-1')).toBeTruthy();
    expect(within(dup).getByText('reason: merged into f-1')).toBeTruthy();
  });

  it('relocated_findings_section', async () => {
    handlers();
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />);
    const relocated = await screen.findByRole('region', { name: 'Relocated findings' });
    expect(within(relocated).getByText('Controller bypasses the guard')).toBeTruthy();
    const published = screen.getByRole('region', { name: 'Published findings' });
    expect(within(published).queryByText('Controller bypasses the guard')).toBeNull();
    expect(
      within(published)
        .getByText('Permission check removed before updating a user')
        .getAttribute('href'),
    ).toBe('/findings/f-1');

    const groups = groupFindings(reviewFindings);
    expect(groups.published.map((f) => f.id)).toEqual(['f-1']);
    expect(groups.relocated.map((f) => f.id)).toEqual(['f-2']);
  });

  it('trace_link_rendered', async () => {
    vi.stubEnv('NEXT_PUBLIC_OPENOBSERVE_UI_URL', 'http://localhost:5080/');
    handlers();
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />);
    const link = await screen.findByRole('link', { name: /4bf92f3577b34da6a3ce929d0e0e4736/ });
    expect(link.getAttribute('href')).toBe(
      'http://localhost:5080/web/traces?trace_id=4bf92f3577b34da6a3ce929d0e0e4736',
    );
    expect(traceUrl('t', '')).toBeNull();
    cleanup();
    renderWithProviders(<TraceLink traceId={null} />);
    expect(screen.getByText('No trace')).toBeTruthy();
  });

  it('no_approve_or_publish_controls', async () => {
    handlers();
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />, { role: 'owner' });
    await screen.findAllByText('Permission check removed before updating a user');
    fireEvent.click(screen.getByRole('button', { name: /Suppressed/ }));
    const buttons = screen.queryAllByRole('button').map((b) => b.textContent ?? '');
    expect(buttons.filter((t) => /approve|publish|merge/i.test(t))).toEqual([]);
  });

  it('early failure shows the failed stage and error class only', async () => {
    handlers(
      reviewDetail({
        state: 'FAILED_INDEXING',
        degraded: false,
        failure: { stage: 'indexing', error_class: 'permanent' },
        stages: [],
        change_summary: null,
        risk: null,
      }),
    );
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />);
    expect(await screen.findByText('Failed: indexing')).toBeTruthy();
    const alert = screen.getByText(/Failed at stage/);
    expect(alert.textContent).toBe('Failed at stage indexing (permanent).');
  });

  it('history selector navigates to another run', async () => {
    handlers();
    renderWithProviders(<ReviewDetailView reviewId={REVIEW_ID} />);
    const select = await screen.findByRole('combobox', { name: 'Review history' });
    fireEvent.change(select, { target: { value: 'run-150' } });
    expect(push).toHaveBeenCalledWith('/reviews/run-150');
  });
});
