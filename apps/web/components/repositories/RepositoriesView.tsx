'use client';

import { useQuery } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { Button } from '@/components/ui/button';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import { repositoriesQuery, repositoryActivityQuery } from '@/lib/queries';
import { canMaintain } from '@/lib/roles';
import { AddRepoDialog } from './AddRepoDialog';
import { RepoTable } from './RepoTable';

export function RepositoriesView() {
  const org = useCurrentOrg();
  if (!org) return <EmptyState title="You are not a member of any organization yet." />;
  return <RepositoriesForOrg orgId={org.id} maintainer={canMaintain(org.role)} />;
}

function RepositoriesForOrg({ orgId, maintainer }: { orgId: string; maintainer: boolean }) {
  const [adding, setAdding] = useState(false);
  const repos = useQuery(repositoriesQuery(orgId));
  const activity = useQuery(repositoryActivityQuery(orgId));
  const activityById = useMemo(
    () => new Map((activity.data ?? []).map((a) => [a.repository_id, a])),
    [activity.data],
  );

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold">Repositories</h1>
          <p className="text-sm text-muted-foreground">
            Repositories enabled for review, with their index state.
          </p>
        </div>
        {maintainer && (
          <Button onClick={() => setAdding(true)}>
            <Plus aria-hidden />
            Add repository
          </Button>
        )}
      </div>

      {repos.isPending && <Loading label="Loading repositories" />}
      {repos.isError && <ErrorState error={repos.error} onRetry={() => void repos.refetch()} />}
      {repos.isSuccess && repos.data.items.length === 0 && (
        <EmptyState title="No repositories yet.">
          {maintainer
            ? 'Use "Add repository" to enable one of your installation repositories.'
            : 'Ask a maintainer to enable a repository.'}
        </EmptyState>
      )}
      {repos.isSuccess && repos.data.items.length > 0 && (
        <RepoTable repositories={repos.data.items} activity={activityById} />
      )}

      {maintainer && <AddRepoDialog orgId={orgId} open={adding} onClose={() => setAdding(false)} />}
    </div>
  );
}
