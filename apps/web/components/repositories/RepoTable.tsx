'use client';

import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { Badge } from '@/components/ui/badge';
import type { Repository, RepositoryActivity } from '@/lib/api/pending';
import { formatDateTime } from '@/lib/format';
import { repositoryStatusQuery } from '@/lib/queries';
import { IndexStateBadge, lastIndexedAt } from './IndexStateBadge';

function LastIndexCell({ repoId }: { repoId: string }) {
  const { data, isPending, isError } = useQuery(repositoryStatusQuery(repoId));
  if (isPending) return <span className="text-muted-foreground">…</span>;
  if (isError || !data) return <span className="text-muted-foreground">-</span>;
  return (
    <span className="flex flex-wrap items-center gap-2">
      <IndexStateBadge state={data.index_state} />
      <span className="text-muted-foreground">{formatDateTime(lastIndexedAt(data))}</span>
    </span>
  );
}

export function RepoTable({
  repositories,
  activity,
}: {
  repositories: Repository[];
  activity?: Map<string, RepositoryActivity>;
}) {
  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50 text-xs text-muted-foreground uppercase">
          <tr>
            <th className="px-3 py-2 font-medium">Name</th>
            <th className="px-3 py-2 font-medium">Provider</th>
            <th className="px-3 py-2 font-medium">Enabled</th>
            <th className="px-3 py-2 font-medium">Last index</th>
            <th className="px-3 py-2 font-medium">Last review</th>
            <th className="px-3 py-2 text-right font-medium">Open PRs</th>
          </tr>
        </thead>
        <tbody>
          {repositories.map((repo) => {
            const act = activity?.get(repo.id);
            return (
              <tr key={repo.id} className="border-t">
                <td className="px-3 py-2 font-medium">
                  <Link href={`/repositories/${repo.id}`} className="hover:underline">
                    {repo.full_name}
                  </Link>
                  {repo.archived && (
                    <Badge variant="muted" className="ml-2">
                      archived
                    </Badge>
                  )}
                </td>
                <td className="px-3 py-2 text-muted-foreground capitalize">{repo.provider}</td>
                <td className="px-3 py-2">
                  {repo.enabled ? (
                    <Badge variant="success">Enabled</Badge>
                  ) : (
                    <Badge variant="muted" title={`access: ${repo.access_state}`}>
                      Disabled
                    </Badge>
                  )}
                </td>
                <td className="px-3 py-2">
                  <LastIndexCell repoId={repo.id} />
                </td>
                <td className="px-3 py-2 text-muted-foreground">
                  {formatDateTime(act?.last_review_at)}
                </td>
                <td className="px-3 py-2 text-right tabular-nums">
                  {act ? act.open_pull_requests : '-'}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
