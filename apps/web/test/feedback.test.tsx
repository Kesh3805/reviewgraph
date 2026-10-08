// @vitest-environment jsdom
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { delay, http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { FeedbackMenu, SUPPRESS_ROLE_MESSAGE } from '../components/feedback/FeedbackMenu';
import { FindingsList } from '../components/review/FindingsList';
import type { FeedbackInput, FindingFeedback } from '../lib/api/pending';
import { applyVerdict, emptyCounts } from '../lib/feedback';
import { reviewFindings } from './fixtures';
import { problem, renderWithProviders, route } from './helpers';

const server = setupServer();
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
});
afterAll(() => server.close());

const initial: FindingFeedback = {
  mine: { verdict: 'useful', comment: null, updated_at: '2026-10-07T10:00:00Z' },
  counts: { ...emptyCounts(), useful: 3, false_positive: 1 },
};

function pressed(name: RegExp) {
  return screen.getByRole('button', { name }).getAttribute('aria-pressed');
}

describe('feedback', () => {
  it('feedback_optimistic_update_and_rollback', async () => {
    server.use(
      http.get(route('/findings/:id/feedback'), () => HttpResponse.json(initial)),
      http.post(route('/findings/:id/feedback'), async () => {
        await delay(50);
        return problem(500, 'database unavailable');
      }),
    );
    renderWithProviders(<FeedbackMenu findingId="f-1" />);
    await waitFor(() => expect(pressed(/^Useful/)).toBe('true'));

    fireEvent.click(screen.getByRole('button', { name: /^False positive/ }));
    // Optimistic: the new verdict and counts show before the server answers.
    await waitFor(() => expect(pressed(/^False positive/)).toBe('true'));
    expect(screen.getByRole('button', { name: /^False positive/ }).textContent).toBe(
      'False positive2',
    );
    expect(screen.getByRole('button', { name: /^Useful/ }).textContent).toBe('Useful2');

    // The server fails: rolled back with a toast.
    expect(await screen.findByText(/Could not save your feedback/)).toBeTruthy();
    expect(pressed(/^Useful/)).toBe('true');
    expect(pressed(/^False positive/)).toBe('false');
    expect(screen.getByRole('button', { name: /^Useful/ }).textContent).toBe('Useful3');

    const next = applyVerdict(initial, 'intentional', 'by design', 't');
    expect(next.counts).toMatchObject({ useful: 2, intentional: 1, false_positive: 1 });
    expect(next.mine).toEqual({ verdict: 'intentional', comment: 'by design', updated_at: 't' });
  });

  it('saves the verdict with a comment and keeps it', async () => {
    let body: FeedbackInput | undefined;
    server.use(
      http.get(route('/findings/:id/feedback'), () =>
        HttpResponse.json({ mine: null, counts: emptyCounts() }),
      ),
      http.post(route('/findings/:id/feedback'), async ({ request }) => {
        body = (await request.json()) as FeedbackInput;
        return HttpResponse.json(applyVerdict(undefined, body.verdict, body.comment ?? null));
      }),
    );
    renderWithProviders(<FeedbackMenu findingId="f-1" />, { role: 'viewer' });
    await screen.findByRole('button', { name: /^Useful0/ });
    fireEvent.click(screen.getByRole('button', { name: 'Comment…' }));
    fireEvent.change(screen.getByLabelText('Feedback comment'), {
      target: { value: '<b>not html</b>' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^Already handled/ }));
    await waitFor(() => expect(body).toBeDefined());
    expect(body).toEqual({ verdict: 'already_handled', comment: '<b>not html</b>' });
    // Rendered as text.
    expect(await screen.findByText(/Your verdict: Already handled/)).toBeTruthy();
    expect(document.querySelector('b')).toBeNull();
  });

  it('suppress_option_maintainer_only', async () => {
    const posts: FeedbackInput[] = [];
    server.use(
      http.get(route('/findings/:id/feedback'), () =>
        HttpResponse.json({ mine: null, counts: emptyCounts() }),
      ),
      http.post(route('/findings/:id/feedback'), async ({ request }) => {
        const body = (await request.json()) as FeedbackInput;
        posts.push(body);
        if (body.create_suppression) return problem(403, 'maintainer role required');
        return HttpResponse.json(applyVerdict(undefined, body.verdict, null));
      }),
    );

    // Viewers never see the option.
    const viewer = renderWithProviders(<FeedbackMenu findingId="f-1" />, { role: 'viewer' });
    fireEvent.click(await screen.findByRole('button', { name: 'Comment…' }));
    expect(screen.queryByLabelText('Suppress future occurrences')).toBeNull();
    viewer.unmount();

    // Maintainers do. A 403 on the suppression still saves the verdict and shows the role message.
    renderWithProviders(<FeedbackMenu findingId="f-1" />, { role: 'member' });
    fireEvent.click(await screen.findByRole('button', { name: 'Comment…' }));
    fireEvent.click(screen.getByLabelText('Suppress future occurrences'));
    fireEvent.change(screen.getByLabelText('Suppress by'), { target: { value: 'symbol' } });
    fireEvent.change(screen.getByLabelText('Suppression reason'), {
      target: { value: 'generated code' },
    });
    fireEvent.click(screen.getByRole('button', { name: /^Intentional/ }));

    expect(await screen.findByText(SUPPRESS_ROLE_MESSAGE)).toBeTruthy();
    expect(posts).toEqual([
      {
        verdict: 'intentional',
        create_suppression: { kind: 'symbol', reason: 'generated code' },
      },
      { verdict: 'intentional' },
    ]);
    expect(pressed(/^Intentional/)).toBe('true');
  });

  it('suppression is only sent with intentional or not relevant', async () => {
    const posts: FeedbackInput[] = [];
    server.use(
      http.get(route('/findings/:id/feedback'), () =>
        HttpResponse.json({ mine: null, counts: emptyCounts() }),
      ),
      http.post(route('/findings/:id/feedback'), async ({ request }) => {
        const body = (await request.json()) as FeedbackInput;
        posts.push(body);
        return HttpResponse.json(applyVerdict(undefined, body.verdict, null));
      }),
    );
    renderWithProviders(<FeedbackMenu findingId="f-1" />, { role: 'owner' });
    fireEvent.click(await screen.findByRole('button', { name: 'Comment…' }));
    fireEvent.click(screen.getByLabelText('Suppress future occurrences'));
    fireEvent.click(screen.getByRole('button', { name: /^Useful/ }));
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(posts[0]).toEqual({ verdict: 'useful' });
  });

  it('finding cards carry the feedback menu', async () => {
    server.use(http.get(route('/findings/:id/feedback'), () => HttpResponse.json(initial)));
    renderWithProviders(<FindingsList findings={reviewFindings} />);
    const card = screen.getByTestId('finding-f-1');
    fireEvent.click(within(card).getByRole('button', { name: 'Give feedback' }));
    expect(await within(card).findByRole('button', { name: /^Useful3/ })).toBeTruthy();
  });
});
