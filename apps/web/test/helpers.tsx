import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import { HttpResponse } from 'msw';
import type { ReactNode } from 'react';
import { OrgProvider, type CurrentOrg } from '../components/org/OrgContext';
import { ToastProvider } from '../components/ui/toast';
import type { MembershipRole } from '../lib/session';

export const ORG_ID = '00000000-0000-4000-8000-000000000001';

export function org(role: MembershipRole = 'member'): CurrentOrg {
  return { id: ORG_ID, displayName: 'Acme', role };
}

/** Renders inside a fresh QueryClient (no retries) and the organization context. */
export function renderWithProviders(
  ui: ReactNode,
  { role = 'member' as MembershipRole | null } = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const result = render(
    <QueryClientProvider client={client}>
      <OrgProvider org={role ? org(role) : null}>
        <ToastProvider>{ui}</ToastProvider>
      </OrgProvider>
    </QueryClientProvider>,
  );
  return { ...result, client };
}

/** An RFC 9457 problem response. */
export function problem(status: number, detail: string, extra: Record<string, unknown> = {}) {
  return HttpResponse.json(
    { type: 'about:blank', title: detail, status, detail, ...extra },
    { status, headers: { 'content-type': 'application/problem+json' } },
  );
}

/** Route pattern for MSW that matches any origin. */
export const route = (path: string) => `*/api/v1${path}`;
