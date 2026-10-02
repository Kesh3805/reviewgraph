import type { Metadata } from 'next';
import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import { Dashboard } from '@/components/dashboard/Dashboard';
import { ORG_COOKIE, resolveOrganization } from '@/lib/org';
import { getSession } from '@/lib/session';

export const metadata: Metadata = { title: 'Dashboard' };

export default async function DashboardPage() {
  const session = await getSession();
  if (!session) redirect('/login');

  // The organization always comes from the session memberships.
  const organization = resolveOrganization(session, (await cookies()).get(ORG_COOKIE)?.value);
  if (!organization) {
    return (
      <div className="max-w-md space-y-2">
        <h1 className="text-2xl font-semibold">No organization yet</h1>
        <p className="text-sm text-muted-foreground">
          Your account is not a member of any organization. Install the GitHub App on an
          organization to get started.
        </p>
      </div>
    );
  }
  return <Dashboard key={organization.id} orgId={organization.id} />;
}
