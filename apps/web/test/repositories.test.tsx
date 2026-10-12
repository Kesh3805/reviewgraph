// @vitest-environment jsdom
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { AddRepoDialog, unenabledRepositories } from '../components/repositories/AddRepoDialog';
import { RepoSettingsForm } from '../components/repositories/RepoSettingsForm';
import { RepositoriesView } from '../components/repositories/RepositoriesView';
import { StatusCard, indexActionError } from '../components/repositories/StatusCard';
import { ApiError } from '../lib/api-client';
import type { Repository, RepositoryStatus } from '../lib/api/pending';
import { INDEX_POLL_MS, isIndexActive, repositoryStatusQuery } from '../lib/queries';
import { parseBranches, toPatch, toFormValues } from '../lib/repo-settings';
import { ORG_ID, problem, renderWithProviders, route } from './helpers';

vi.mock('next/navigation', () => ({ usePathname: () => '/repositories' }));

const REPO_ID = '00000000-0000-4000-8000-0000000000a1';

const repository: Repository = {
  id: REPO_ID,
  organization_id: ORG_ID,
  provider: 'github',
  provider_repo_id: '101',
  full_name: 'acme/api',
  default_branch: 'main',
  visibility: 'private',
  archived: false,
  enabled: true,
  access_state: 'active',
  primary_language: 'TypeScript',
  initialized_at: '2026-10-01T10:00:00Z',
  settings: {
    enabled: true,
    target_branches: ['main'],
    skip_drafts: true,
    skip_bots: true,
    reviewer_overrides: { security: true },
  },
  created_at: '2026-09-01T10:00:00Z',
  updated_at: '2026-10-01T10:00:00Z',
};

function status(overrides: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    repository_id: REPO_ID,
    index_state: 'ready',
    init: {
      commit_sha: 'abcdef1234567',
      fingerprint: 'fp-123',
      facts_hash: 'h',
      facts_schema_version: 1,
      tool_version: '0.4.0',
      primary_language: 'TypeScript',
      is_monorepo: false,
      frameworks: ['nestjs'],
      warnings_count: 0,
      detected_at: '2026-10-01T10:00:00Z',
    },
    last_snapshot: {
      full: { id: 's1', commit_sha: 'abcdef1234567', created_at: '2026-10-02T10:00:00Z' },
      delta: null,
    },
    active_job: null,
    config: { hash: 'cfg-1', validation_errors: [] },
    profile_computed_at: null,
    ...overrides,
  };
}

const server = setupServer();
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
});
afterAll(() => server.close());

function listHandlers() {
  server.use(
    http.get(route('/repositories'), ({ request }) => {
      expect(new URL(request.url).searchParams.get('organization_id')).toBe(ORG_ID);
      return HttpResponse.json({ items: [repository], next_cursor: null });
    }),
    http.get(route('/repositories/:id/status'), () => HttpResponse.json(status())),
    http.get(route('/organizations/:id/repository-activity'), () =>
      HttpResponse.json({
        items: [
          { repository_id: REPO_ID, last_review_at: '2026-10-07T09:30:00Z', open_pull_requests: 4 },
        ],
      }),
    ),
  );
}

describe('repositories', () => {
  it('repo_table_renders', async () => {
    listHandlers();
    renderWithProviders(<RepositoriesView />);

    const link = await screen.findByRole('link', { name: 'acme/api' });
    expect(link.getAttribute('href')).toBe(`/repositories/${REPO_ID}`);
    const row = link.closest('tr') as HTMLElement;
    expect(within(row).getByText('github')).toBeTruthy();
    expect(within(row).getByText('Enabled')).toBeTruthy();
    expect(await within(row).findByText('Indexed')).toBeTruthy();
    expect(within(row).getByText('2026-10-02 10:00 UTC')).toBeTruthy();
    expect(await within(row).findByText('2026-10-07 09:30 UTC')).toBeTruthy();
    expect(within(row).getByText('4')).toBeTruthy();
  });

  it('add_repo_dialog_lists_unenabled', async () => {
    let created: unknown;
    server.use(
      http.get(route('/installations/repositories'), () =>
        HttpResponse.json({
          items: [
            { installation_id: 'i1', full_name: 'acme/api', private: true, enabled: true },
            { installation_id: 'i1', full_name: 'acme/web', private: false, enabled: false },
            { installation_id: 'i1', full_name: 'acme/billing', private: true, enabled: false },
          ],
        }),
      ),
      http.post(route('/repositories'), async ({ request }) => {
        created = await request.json();
        return HttpResponse.json({ ...repository, full_name: 'acme/web' }, { status: 201 });
      }),
    );
    const onClose = vi.fn();
    renderWithProviders(<AddRepoDialog orgId={ORG_ID} open onClose={onClose} />);

    const list = await screen.findByRole('list', { name: 'Installation repositories' });
    const names = within(list)
      .getAllByRole('listitem')
      .map((li) => li.firstChild?.textContent);
    expect(names).toEqual(['acme/billingprivate', 'acme/web']);
    expect(within(list).queryByText('acme/api')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Enable acme/web' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(created).toEqual({ installation_id: 'i1', full_name: 'acme/web' });

    expect(
      unenabledRepositories([
        { installation_id: 'i', full_name: 'b', private: false, enabled: false },
        { installation_id: 'i', full_name: 'a', private: false, enabled: true },
      ]).map((r) => r.full_name),
    ).toEqual(['b']);
  });

  it('initialize_disabled_while_running', async () => {
    server.use(
      http.get(route('/repositories/:id/status'), () =>
        HttpResponse.json(
          status({
            index_state: 'indexing',
            active_job: { id: 'job-7', queue: 'repository-index', state: 'running' },
          }),
        ),
      ),
    );
    renderWithProviders(<StatusCard repoId={REPO_ID} maintainer />);

    expect(await screen.findByText('Indexing')).toBeTruthy();
    const init = screen.getByRole('button', { name: 'Initialize' }) as HTMLButtonElement;
    const rebuild = screen.getByRole('button', { name: 'Rebuild graph' }) as HTMLButtonElement;
    expect(init.disabled).toBe(true);
    expect(rebuild.disabled).toBe(true);
    expect(screen.getByText('job-7 (running)')).toBeTruthy();

    // The status card polls every 5 s only while a job is active.
    expect(isIndexActive(status({ index_state: 'indexing' }))).toBe(true);
    expect(isIndexActive(status())).toBe(false);
    const interval = repositoryStatusQuery(REPO_ID).refetchInterval as (q: {
      state: { data: RepositoryStatus | undefined };
    }) => number | false;
    expect(interval({ state: { data: status({ index_state: 'queued' }) } })).toBe(INDEX_POLL_MS);
    expect(interval({ state: { data: status() } })).toBe(false);
  });

  it('initialize shows the existing job on 409', async () => {
    server.use(
      http.get(route('/repositories/:id/status'), () => HttpResponse.json(status())),
      http.post(route('/repositories/:id/initialize'), () =>
        problem(409, 'an index job already exists for this head', { job_id: 'job-42' }),
      ),
    );
    renderWithProviders(<StatusCard repoId={REPO_ID} maintainer />);
    const init = await screen.findByRole('button', { name: 'Initialize' });
    await waitFor(() => expect((init as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(init);
    expect(await screen.findByText('Index already running (job job-42).')).toBeTruthy();
    expect(indexActionError(new ApiError(403, { status: 403 }))).toBe('Requires maintainer role.');
  });

  it('viewer_sees_no_actions', async () => {
    listHandlers();
    server.use(http.get(route('/repositories/:id/status'), () => HttpResponse.json(status())));
    renderWithProviders(
      <>
        <RepositoriesView />
        <StatusCard repoId={REPO_ID} maintainer={false} />
        <RepoSettingsForm repository={repository} maintainer={false} />
      </>,
      { role: 'viewer' },
    );
    await screen.findByRole('link', { name: 'acme/api' });
    await screen.findAllByText('fp-123');
    expect(screen.queryByRole('button', { name: /add repository/i })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Initialize' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Rebuild graph' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Save settings' })).toBeNull();
    const fieldset = screen.getByRole('group') as HTMLFieldSetElement;
    expect(fieldset.disabled).toBe(true);
  });

  it('settings_form_validation', async () => {
    let patched: unknown;
    server.use(
      http.patch(route('/repositories/:id/settings'), async ({ request }) => {
        patched = await request.json();
        return HttpResponse.json({
          ...repository.settings,
          target_branches: ['main', 'release/*'],
        });
      }),
    );
    renderWithProviders(<RepoSettingsForm repository={repository} maintainer />);

    const branches = screen.getByLabelText('Target branches');
    fireEvent.change(branches, { target: { value: 'main, release /x' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    expect(await screen.findByText('"release /x" must not contain whitespace.')).toBeTruthy();
    expect(patched).toBeUndefined();

    fireEvent.change(branches, { target: { value: 'main\nrelease/*' } });
    fireEvent.change(screen.getByLabelText('performance reviewer'), { target: { value: 'off' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    await waitFor(() => expect(patched).toBeDefined());
    expect(patched).toEqual({
      enabled: true,
      skip_drafts: true,
      skip_bots: true,
      target_branches: ['main', 'release/*'],
      reviewer_overrides: { security: true, performance: false },
    });

    expect(parseBranches(' main ,\n\n dev ')).toEqual(['main', 'dev']);
    expect(toPatch(toFormValues(repository.settings)).reviewer_overrides).toEqual({
      security: true,
    });
  });
});
