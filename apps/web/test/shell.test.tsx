// @vitest-environment jsdom
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { OrgSwitcher } from '../components/shell/OrgSwitcher';
import { NAV_ITEMS, isActive } from '../components/shell/nav';
import { ORG_COOKIE, resolveOrganization } from '../lib/org';
import type { SessionOrganization } from '../lib/session';

const refresh = vi.fn();
vi.mock('next/navigation', () => ({
  useRouter: () => ({ refresh }),
  usePathname: () => '/',
}));

const orgs: SessionOrganization[] = [
  { id: 'o1', slug: 'acme', display_name: 'Acme', role: 'owner' },
  { id: 'o2', slug: 'globex', display_name: 'Globex', role: 'member' },
];

afterEach(() => {
  cleanup();
  refresh.mockClear();
});

describe('shell', () => {
  it('org_switcher_lists_memberships', () => {
    render(
      <QueryClientProvider client={new QueryClient()}>
        <OrgSwitcher organizations={orgs} currentId="o2" />
      </QueryClientProvider>,
    );
    const select = screen.getByRole('combobox', { name: 'Organization' }) as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => o.textContent)).toEqual(['Acme', 'Globex']);
    expect(select.value).toBe('o2');

    fireEvent.change(select, { target: { value: 'o1' } });
    expect(document.cookie).toContain(`${ORG_COOKIE}=o1`);
    expect(refresh).toHaveBeenCalledTimes(1);
  });

  it('only resolves organizations the user is a member of', () => {
    const session = { organizations: orgs, current_organization_id: 'o2' };
    expect(resolveOrganization(session, 'o1')?.id).toBe('o1');
    expect(resolveOrganization(session, 'someone-elses-org')?.id).toBe('o2');
    expect(resolveOrganization({ ...session, current_organization_id: null })?.id).toBe('o1');
    expect(resolveOrganization({ organizations: [], current_organization_id: null })).toBeNull();
  });

  it('navigation covers every section in order', () => {
    expect(NAV_ITEMS.map((i) => i.label)).toEqual([
      'Dashboard',
      'Repositories',
      'Pull Requests',
      'Rules',
      'Integrations',
      'Usage',
      'Settings',
    ]);
    expect(isActive('/', '/')).toBe(true);
    expect(isActive('/repositories/abc', '/repositories')).toBe(true);
    expect(isActive('/repositories', '/')).toBe(false);
  });
});
