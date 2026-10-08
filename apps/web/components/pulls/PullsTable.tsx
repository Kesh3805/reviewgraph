'use client';

import { useMutation, useQueryClient } from '@tanstack/react-query';
import Link from 'next/link';
import { Button } from '@/components/ui/button';
import { errorMessage } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { triggerManualReview } from '@/lib/api/endpoints';
import type { PullRequestSummary } from '@/lib/api/pending';
import { formatDateTime, shortSha } from '@/lib/format';
import { RunStateBadge } from './RunStateBadge';
import { SeverityCounts } from './SeverityCounts';

function manualReviewError(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 403) return 'Requires maintainer role.';
    if (error.status === 409) return 'The pull request is closed.';
  }
  return errorMessage(error, 'Could not request a review.');
}

/** Maintainer-only re-review at the current head (there is deliberately no "retry"). */
function ManualReviewButton({ pr }: { pr: PullRequestSummary }) {
  const queryClient = useQueryClient();
  const review = useMutation({
    mutationFn: () => triggerManualReview(pr.id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['pull-requests'] }),
  });
  return (
    <span className="flex flex-col items-end gap-1">
      <Button
        size="sm"
        variant="outline"
        disabled={review.isPending || pr.state !== 'open'}
        onClick={() => review.mutate()}
        aria-label={`Review #${pr.number} now`}
      >
        {review.isPending ? 'Requesting…' : 'Review now'}
      </Button>
      {review.isSuccess && (
        <span role="status" className="text-xs text-muted-foreground">
          Review requested.
        </span>
      )}
      {review.isError && (
        <span role="alert" className="text-xs text-destructive">
          {manualReviewError(review.error)}
        </span>
      )}
    </span>
  );
}

export function PullsTable({
  pulls,
  maintainer,
  showRepository,
}: {
  pulls: PullRequestSummary[];
  maintainer: boolean;
  showRepository: boolean;
}) {
  return (
    <div className="overflow-x-auto rounded-md border">
      <table className="w-full text-left text-sm">
        <thead className="bg-muted/50 text-xs text-muted-foreground uppercase">
          <tr>
            <th className="px-3 py-2 font-medium">Pull request</th>
            <th className="px-3 py-2 font-medium">Author</th>
            <th className="px-3 py-2 font-medium">Head</th>
            <th className="px-3 py-2 font-medium">Latest run</th>
            <th className="px-3 py-2 font-medium">Findings</th>
            <th className="px-3 py-2 font-medium">Updated</th>
            {maintainer && (
              <th className="px-3 py-2 font-medium">
                <span className="sr-only">Actions</span>
              </th>
            )}
          </tr>
        </thead>
        <tbody>
          {pulls.map((pr) => {
            const title = `#${pr.number} ${pr.title}`;
            return (
              <tr key={pr.id} className="border-t align-top">
                <td className="max-w-md px-3 py-2">
                  {showRepository && (
                    <span className="block text-xs text-muted-foreground">
                      {pr.repository_full_name}
                    </span>
                  )}
                  {pr.latest_run ? (
                    <Link href={`/reviews/${pr.latest_run.id}`} className="hover:underline">
                      {title}
                    </Link>
                  ) : (
                    <span>{title}</span>
                  )}
                  {pr.draft && <span className="ml-2 text-xs text-muted-foreground">draft</span>}
                </td>
                <td className="px-3 py-2 text-muted-foreground">{pr.author}</td>
                <td className="px-3 py-2 font-mono text-xs">{shortSha(pr.head_sha)}</td>
                <td className="px-3 py-2">
                  {pr.latest_run ? (
                    <RunStateBadge state={pr.latest_run.state} degraded={pr.latest_run.degraded} />
                  ) : (
                    <span className="text-muted-foreground">Not reviewed</span>
                  )}
                </td>
                <td className="px-3 py-2">
                  <SeverityCounts counts={pr.findings_by_severity} />
                </td>
                <td className="px-3 py-2 whitespace-nowrap text-muted-foreground">
                  {formatDateTime(pr.updated_at)}
                </td>
                {maintainer && (
                  <td className="px-3 py-2 text-right">
                    <ManualReviewButton pr={pr} />
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
