'use client';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Dialog } from '@/components/ui/dialog';
import { EmptyState, ErrorState, Loading, errorMessage } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { enableRepository } from '@/lib/api/endpoints';
import type { InstallationRepository } from '@/lib/api/pending';
import { installationRepositoriesQuery, keys } from '@/lib/queries';

/** Installation repositories that are not enabled yet, sorted by name. */
export function unenabledRepositories(items: InstallationRepository[]): InstallationRepository[] {
  return items.filter((r) => !r.enabled).sort((a, b) => a.full_name.localeCompare(b.full_name));
}

function enableError(error: unknown): string {
  if (error instanceof ApiError && error.status === 403) return 'Requires maintainer role.';
  return errorMessage(error, 'Could not enable the repository.');
}

export function AddRepoDialog({
  orgId,
  open,
  onClose,
}: {
  orgId: string;
  open: boolean;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const query = useQuery({ ...installationRepositoriesQuery(orgId), enabled: open });
  const [pendingName, setPendingName] = useState<string | null>(null);

  const enable = useMutation({
    mutationFn: (repo: InstallationRepository) =>
      enableRepository({ installation_id: repo.installation_id, full_name: repo.full_name }),
    onMutate: (repo) => setPendingName(repo.full_name),
    onSettled: () => setPendingName(null),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: keys.repositories(orgId) }),
        queryClient.invalidateQueries({ queryKey: keys.installationRepositories(orgId) }),
      ]);
      onClose();
    },
  });

  const candidates = query.data ? unenabledRepositories(query.data) : [];

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Add repository"
      description="Repositories your GitHub App installations can access that are not enabled yet."
    >
      {query.isPending && <Loading label="Loading installation repositories" />}
      {query.isError && <ErrorState error={query.error} onRetry={() => void query.refetch()} />}
      {query.isSuccess && candidates.length === 0 && (
        <EmptyState title="Every installation repository is already enabled.">
          Grant the GitHub App access to more repositories to add them here.
        </EmptyState>
      )}
      {candidates.length > 0 && (
        <ul aria-label="Installation repositories" className="divide-y rounded-md border">
          {candidates.map((repo) => (
            <li
              key={`${repo.installation_id}:${repo.full_name}`}
              className="flex items-center justify-between gap-2 px-3 py-2 text-sm"
            >
              <span className="flex items-center gap-2">
                {repo.full_name}
                {repo.private && <Badge variant="muted">private</Badge>}
              </span>
              <Button
                size="sm"
                disabled={enable.isPending}
                onClick={() => enable.mutate(repo)}
                aria-label={`Enable ${repo.full_name}`}
              >
                {pendingName === repo.full_name ? 'Enabling…' : 'Enable'}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {enable.isError && (
        <p role="alert" className="mt-3 text-sm text-destructive">
          {enableError(enable.error)}
        </p>
      )}
    </Dialog>
  );
}
