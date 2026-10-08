'use client';

import { useQueryClient } from '@tanstack/react-query';
import { useRouter } from 'next/navigation';
import { ORG_COOKIE } from '@/lib/org';
import type { SessionOrganization } from '@/lib/session';

/** Lists only the user's memberships; the API re-validates membership on every request. */
export function OrgSwitcher({
  organizations,
  currentId,
}: {
  organizations: SessionOrganization[];
  currentId: string | null;
}) {
  const router = useRouter();
  const queryClient = useQueryClient();

  function select(id: string) {
    // Preference only (not a credential): the server re-resolves it against the session.
    document.cookie = `${ORG_COOKIE}=${encodeURIComponent(id)}; path=/; max-age=31536000; samesite=lax`;
    queryClient.clear();
    router.refresh();
  }

  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="sr-only text-muted-foreground md:not-sr-only">Organization</span>
      <select
        aria-label="Organization"
        className="h-9 rounded-md border bg-background px-2 text-sm shadow-xs"
        value={currentId ?? ''}
        disabled={organizations.length === 0}
        onChange={(e) => select(e.target.value)}
      >
        {organizations.map((org) => (
          <option key={org.id} value={org.id}>
            {org.display_name}
          </option>
        ))}
      </select>
    </label>
  );
}
