'use client';

import { useQuery } from '@tanstack/react-query';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import {
  parsePullFilters,
  pullFiltersToSearch,
  toListQuery,
  type PullFilters,
} from '@/lib/pull-filters';
import { pullRequestsQuery, repositoriesQuery } from '@/lib/queries';
import { canMaintain } from '@/lib/roles';
import { Filters } from './Filters';
import { PullsTable } from './PullsTable';

/**
 * Pull request list, organization-wide (`/pull-requests`) or for one repository
 * (`/repositories/[repoId]/pulls`). Filters and the cursor live in the URL.
 */
export function PullsView({ repoId }: { repoId?: string }) {
  const org = useCurrentOrg();
  if (!org) return <EmptyState title="You are not a member of any organization yet." />;
  return <PullsForOrg orgId={org.id} maintainer={canMaintain(org.role)} repoId={repoId} />;
}

function PullsForOrg({
  orgId,
  maintainer,
  repoId,
}: {
  orgId: string;
  maintainer: boolean;
  repoId?: string;
}) {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const filters = parsePullFilters(searchParams);
  const pulls = useQuery(pullRequestsQuery(toListQuery(filters, { orgId, repoId })));
  const repos = useQuery({ ...repositoriesQuery(orgId), enabled: !repoId, throwOnError: false });

  const navigate = (next: PullFilters, push = false) => {
    const url = `${pathname}${pullFiltersToSearch(next)}`;
    if (push) router.push(url);
    else router.replace(url);
  };

  return (
    <div className="space-y-4">
      {!repoId && (
        <div>
          <h1 className="text-2xl font-semibold">Pull Requests</h1>
          <p className="text-sm text-muted-foreground">
            Pull requests and the state of their latest review.
          </p>
        </div>
      )}
      <Filters
        filters={filters}
        onChange={(next) => navigate(next)}
        repositories={repoId ? undefined : (repos.data?.items ?? [])}
      />

      {pulls.isPending && <Loading label="Loading pull requests" />}
      {pulls.isError && <ErrorState error={pulls.error} onRetry={() => void pulls.refetch()} />}
      {pulls.isSuccess && pulls.data.items.length === 0 && (
        <EmptyState title="No pull requests match these filters." />
      )}
      {pulls.isSuccess && pulls.data.items.length > 0 && (
        <PullsTable pulls={pulls.data.items} maintainer={maintainer} showRepository={!repoId} />
      )}

      <div className="flex justify-end gap-2">
        {filters.cursor && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => navigate({ ...filters, cursor: undefined }, true)}
          >
            First page
          </Button>
        )}
        {pulls.data?.next_cursor && (
          <Button
            variant="outline"
            size="sm"
            onClick={() =>
              navigate({ ...filters, cursor: pulls.data.next_cursor ?? undefined }, true)
            }
          >
            Next page
          </Button>
        )}
      </div>
    </div>
  );
}
