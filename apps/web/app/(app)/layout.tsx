import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import type { ReactNode } from 'react';
import { OrgProvider } from '@/components/org/OrgContext';
import { OrgSwitcher } from '@/components/shell/OrgSwitcher';
import { Sidebar } from '@/components/shell/Sidebar';
import { UserMenu } from '@/components/shell/UserMenu';
import { ORG_COOKIE, resolveOrganization } from '@/lib/org';
import { getSession } from '@/lib/session';

/** Authenticated shell: sidebar navigation, organization switcher and user menu. */
export default async function AppLayout({ children }: { children: ReactNode }) {
  const session = await getSession();
  if (!session) redirect('/login');

  const preferred = (await cookies()).get(ORG_COOKIE)?.value;
  const organization = resolveOrganization(session, preferred);

  return (
    <div className="flex min-h-screen flex-col md:flex-row">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center justify-between gap-4 border-b px-4 py-3 md:px-8">
          <OrgSwitcher organizations={session.organizations} currentId={organization?.id ?? null} />
          <UserMenu user={session.user} />
        </header>
        <main className="flex-1 p-4 md:p-8">
          <OrgProvider
            org={
              organization
                ? {
                    id: organization.id,
                    displayName: organization.display_name,
                    role: organization.role,
                  }
                : null
            }
          >
            {children}
          </OrgProvider>
        </main>
      </div>
    </div>
  );
}
