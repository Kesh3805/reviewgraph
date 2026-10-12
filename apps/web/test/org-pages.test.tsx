// @vitest-environment jsdom
import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { IntegrationsView } from '../components/integrations/IntegrationsView';
import { RulesView } from '../components/rules/RulesView';
import { CONFLICT_MESSAGE, SettingsView } from '../components/settings/SettingsView';
import { UsageView } from '../components/usage/UsageView';
import type {
  AuditEntry,
  GithubIntegration,
  OrganizationSettings,
  Suppression,
  UsageReport,
} from '../lib/api/pending';
import { ORG_ID, problem, renderWithProviders, route } from './helpers';

vi.mock('../lib/highlight', () => ({ highlight: vi.fn(async () => null) }));

const server = setupServer();
beforeAll(() => server.listen());
afterEach(() => {
  cleanup();
  server.resetHandlers();
});
afterAll(() => server.close());

const REPO = 'repo-1';

function rulesHandlers() {
  const suppressions: Suppression[] = [
    {
      id: 'sup-1',
      kind: 'path',
      value: 'src/generated/**',
      reason: 'generated code',
      created_by: 'octocat',
      created_at: '2026-10-01T10:00:00Z',
      revoked_at: null,
      revoked_by: null,
    },
  ];
  const audit: AuditEntry[] = [
    {
      id: 'a1',
      action: 'suppression.created',
      actor: 'octocat',
      target: 'sup-1',
      at: '2026-10-01T10:00:00Z',
    },
  ];
  server.use(
    http.get(route('/repositories/:id/status'), () =>
      HttpResponse.json({
        repository_id: REPO,
        index_state: 'ready',
        init: null,
        last_snapshot: null,
        active_job: null,
        config: {
          hash: 'cfg-1',
          validation_errors: [
            'reviewers.security: unknown key "depth"',
            'conventions[0].scope: invalid glob',
          ],
        },
        profile_computed_at: null,
      }),
    ),
    http.get(route('/repositories/:id/rules'), () =>
      HttpResponse.json({
        config: { path: '.review/config.yaml', yaml: 'reviewers:\n  security:\n    depth: deep\n' },
        rules: [
          {
            id: 'no-raw-sql',
            description: 'Use the query builder',
            severity: 'high',
            violations_30d: 3,
          },
        ],
      }),
    ),
    http.get(route('/repositories/:id/suppressions'), () =>
      HttpResponse.json({ items: suppressions, audit }),
    ),
    http.delete(route('/repositories/:id/suppressions/:sid'), ({ params }) => {
      const s = suppressions.find((x) => x.id === params.sid);
      if (!s) return problem(404, 'not found');
      s.revoked_at = '2026-10-08T10:00:00Z';
      s.revoked_by = 'maintainer';
      audit.push({
        id: 'a2',
        action: 'suppression.revoked',
        actor: 'maintainer',
        target: s.id,
        at: s.revoked_at,
      });
      return HttpResponse.json(s);
    }),
  );
}

describe('rules, integrations, usage and settings', () => {
  it('rules_page_shows_validation_errors', async () => {
    rulesHandlers();
    renderWithProviders(<RulesView repoId={REPO} />);
    expect(await screen.findByText('2 validation errors')).toBeTruthy();
    expect(screen.getByText('reviewers.security: unknown key "depth"')).toBeTruthy();
    expect((await screen.findByTestId('config-yaml')).textContent).toContain('depth: deep');
    expect(screen.getByText('no-raw-sql')).toBeTruthy();
    // Read-only: the config cannot be edited here.
    expect(screen.queryByRole('textbox', { name: /config/i })).toBeNull();
  });

  it('suppression_revoke_audited', async () => {
    rulesHandlers();
    renderWithProviders(<RulesView repoId={REPO} />, { role: 'member' });
    fireEvent.click(
      await screen.findByRole('button', { name: 'Revoke suppression src/generated/**' }),
    );
    const row = screen.getByTestId('suppression-sup-1');
    expect(await within(row).findByText('Revoked')).toBeTruthy();
    const history = screen.getByRole('list', { name: 'Audit history' });
    expect(within(history).getByText('suppression.revoked')).toBeTruthy();
    cleanup();

    // Viewers cannot create or revoke.
    rulesHandlers();
    renderWithProviders(<RulesView repoId={REPO} />, { role: 'viewer' });
    await screen.findByText('src/generated/**');
    expect(screen.queryByRole('button', { name: /revoke/i })).toBeNull();
    expect(screen.queryByRole('form', { name: 'Create suppression' })).toBeNull();
  });

  it('integrations_shows_permission_check', async () => {
    const data: GithubIntegration = {
      installation: {
        id: 'inst-1',
        account_login: 'acme',
        state: 'active',
        installed_at: '2026-09-01T10:00:00Z',
      },
      permissions_check: {
        ok: false,
        checked_at: '2026-10-08T09:00:00Z',
        missing: ['checks:write'],
        excess: ['contents:write'],
      },
      webhooks: {
        last_delivery_at: '2026-10-08T09:59:00Z',
        last_delivery_event: 'pull_request',
        deliveries_24h: 42,
        signature_failures_24h: 3,
      },
      reconciler: { state: 'lagging', last_run_at: '2026-10-08T09:30:00Z', repaired_24h: 2 },
    };
    server.use(
      http.get(route('/organizations/:id/integrations/github'), () => HttpResponse.json(data)),
    );
    renderWithProviders(<IntegrationsView />);
    const check = await screen.findByTestId('permission-check');
    expect(within(check).getByText('Permission check failed')).toBeTruthy();
    expect(screen.getByText('checks:write')).toBeTruthy();
    expect(screen.getByText('contents:write')).toBeTruthy();
    expect(screen.getByText('lagging')).toBeTruthy();
    expect(screen.getByText('3')).toBeTruthy();
  });

  it('usage_groups_by_tier', async () => {
    const seen: string[] = [];
    server.use(
      http.get(route('/organizations/:id/usage'), ({ request, params }) => {
        expect(params.id).toBe(ORG_ID);
        const url = new URL(request.url);
        const groupBy = url.searchParams.get('group_by') ?? '';
        seen.push(groupBy);
        const report: UsageReport = {
          from: url.searchParams.get('from') ?? '',
          to: url.searchParams.get('to') ?? '',
          group_by: groupBy as UsageReport['group_by'],
          rows:
            groupBy === 'tier'
              ? [
                  {
                    key: 'tier-1',
                    model_calls: 10,
                    input_tokens: 1000,
                    output_tokens: 100,
                    cached_tokens: 0,
                    cost_usd_micros: 20_000,
                  },
                  {
                    key: 'tier-2',
                    model_calls: 4,
                    input_tokens: 8000,
                    output_tokens: 900,
                    cached_tokens: 2000,
                    cost_usd_micros: 1_500_000,
                  },
                ]
              : [
                  {
                    key: '2026-10-07',
                    model_calls: 14,
                    input_tokens: 9000,
                    output_tokens: 1000,
                    cached_tokens: 2000,
                    cost_usd_micros: 1_520_000,
                  },
                ],
          totals: {
            model_calls: 14,
            input_tokens: 9000,
            output_tokens: 1000,
            cached_tokens: 2000,
            cost_usd_micros: 1_520_000,
            reviewed_prs: 4,
            useful_findings: 2,
          },
          cost_per_reviewed_pr_usd_micros: 380_000,
          cost_per_useful_finding_usd_micros: 760_000,
        };
        return HttpResponse.json(report);
      }),
    );
    renderWithProviders(<UsageView />);
    expect(await screen.findByText('2026-10-07')).toBeTruthy();
    expect(screen.getByTestId('usage-per-finding').textContent).toBe('$0.7600');
    expect(screen.getByTestId('usage-cost').textContent).toBe('$1.52');

    fireEvent.change(screen.getByLabelText('Group by'), { target: { value: 'tier' } });
    const table = await screen.findByRole('table', { name: 'Usage' });
    await waitFor(() => expect(within(table).getByText('tier-2')).toBeTruthy());
    expect(within(table).getByText('tier-1')).toBeTruthy();
    expect(within(table).getByText('Model tier')).toBeTruthy();
    expect(seen).toEqual(['day', 'tier']);
  });

  const settings: OrganizationSettings = {
    retention: { source_days: 30, artifacts_days: 90 },
    external_models: false,
    updated_at: '2026-10-01T10:00:00.000Z',
  };

  function settingsHandlers(patch = () => HttpResponse.json(settings) as Response) {
    server.use(
      http.get(route('/organizations/:id/settings'), () => HttpResponse.json(settings)),
      http.patch(route('/organizations/:id/settings'), patch),
      http.get(route('/organizations/:id/members'), () =>
        HttpResponse.json({
          items: [{ user_id: 'u1', login: 'octocat', display_name: 'Octo Cat', role: 'admin' }],
        }),
      ),
    );
  }

  it('settings_admin_only', async () => {
    settingsHandlers();
    renderWithProviders(<SettingsView />, { role: 'member' });
    expect(await screen.findByText('External models are disabled.')).toBeTruthy();
    expect(screen.getByText('Only organization admins can change these settings.')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Save settings' })).toBeNull();
    expect(screen.getByLabelText('Source retention (days)').matches(':disabled')).toBe(true);
    expect(await screen.findByText('admin')).toBeTruthy();
    expect(screen.queryByRole('combobox', { name: 'Role of octocat' })).toBeNull();
    cleanup();

    settingsHandlers();
    renderWithProviders(<SettingsView />, { role: 'admin' });
    expect(await screen.findByRole('button', { name: 'Save settings' })).toBeTruthy();
    expect(await screen.findByRole('combobox', { name: 'Role of octocat' })).toBeTruthy();
  });

  it('settings_conflict_409', async () => {
    let body: unknown;
    settingsHandlers(() => problem(409, 'settings changed'));
    server.use(
      http.patch(route('/organizations/:id/settings'), async ({ request }) => {
        body = await request.json();
        return problem(409, 'settings changed');
      }),
    );
    renderWithProviders(<SettingsView />, { role: 'owner' });
    const input = await screen.findByLabelText('Source retention (days)');
    fireEvent.change(input, { target: { value: '14' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save settings' }));
    expect(await screen.findByText(CONFLICT_MESSAGE)).toBeTruthy();
    expect(body).toEqual({
      retention: { source_days: 14, artifacts_days: 90 },
      external_models: false,
      updated_at: '2026-10-01T10:00:00.000Z',
    });
    expect(screen.getByRole('button', { name: 'Reload' })).toBeTruthy();
  });
});
