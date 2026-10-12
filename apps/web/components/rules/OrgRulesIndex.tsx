'use client';

import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import { repositoriesQuery } from '@/lib/queries';

/** `/rules`: rules are per repository, so this lists each repository's Rules tab. */
export function OrgRulesIndex() {
  const org = useCurrentOrg();
  const repos = useQuery({ ...repositoriesQuery(org?.id ?? ''), enabled: !!org });
  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-semibold">Rules</h1>
        <p className="text-sm text-muted-foreground">
          Each repository owns its <code>.review/config.yaml</code>. Pick a repository to see its
          effective configuration, rules and suppressions.
        </p>
      </div>
      {!org && <EmptyState title="You are not a member of any organization yet." />}
      {repos.isPending && org && <Loading label="Loading repositories" />}
      {repos.isError && <ErrorState error={repos.error} onRetry={() => void repos.refetch()} />}
      {repos.data && (
        <ul className="divide-y rounded-md border text-sm">
          {repos.data.items.map((r) => (
            <li key={r.id} className="px-3 py-2">
              <Link href={`/repositories/${r.id}/rules`} className="hover:underline">
                {r.full_name}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
