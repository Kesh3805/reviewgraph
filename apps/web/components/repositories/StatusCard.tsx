'use client';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { ErrorState, Loading, errorMessage } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { initializeRepository, rebuildRepositoryGraph } from '@/lib/api/endpoints';
import { formatDateTime, shortSha } from '@/lib/format';
import { isIndexActive, keys, repositoryStatusQuery } from '@/lib/queries';
import { IndexStateBadge } from './IndexStateBadge';

/** User-facing message for a failed initialize/rebuild. */
export function indexActionError(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 409) {
      const job = typeof error.problem.job_id === 'string' ? error.problem.job_id : 'unknown';
      return `Index already running (job ${job}).`;
    }
    if (error.status === 403) return 'Requires maintainer role.';
    if (error.status === 429) {
      const after = error.problem.retry_after_seconds;
      const minutes = typeof after === 'number' ? Math.ceil(after / 60) : null;
      return minutes
        ? `A rebuild was already requested this hour. Try again in ${minutes} min.`
        : 'A rebuild was already requested this hour.';
    }
  }
  return errorMessage(error, 'The request failed.');
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap justify-between gap-2 py-1">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="text-right font-mono text-xs break-all">{children}</dd>
    </div>
  );
}

export function StatusCard({ repoId, maintainer }: { repoId: string; maintainer: boolean }) {
  const queryClient = useQueryClient();
  const status = useQuery(repositoryStatusQuery(repoId));
  const refreshStatus = () =>
    queryClient.invalidateQueries({ queryKey: keys.repositoryStatus(repoId) });

  const initialize = useMutation({
    mutationFn: () => initializeRepository(repoId),
    onSettled: refreshStatus,
  });
  const rebuild = useMutation({
    mutationFn: () => rebuildRepositoryGraph(repoId),
    onSettled: refreshStatus,
  });

  const data = status.data;
  const running = isIndexActive(data);
  const busy = running || initialize.isPending || rebuild.isPending;
  const actionError = initialize.error ?? rebuild.error;

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-center justify-between gap-2">
        <CardTitle>Index status</CardTitle>
        {data && <IndexStateBadge state={data.index_state} />}
      </CardHeader>
      <CardContent className="space-y-4 text-sm">
        {status.isPending && <Loading label="Loading index status" className="h-24" />}
        {status.isError && (
          <ErrorState error={status.error} onRetry={() => void status.refetch()} />
        )}
        {data && (
          <dl className="divide-y">
            <Row label="Fingerprint">{data.init?.fingerprint ?? '-'}</Row>
            <Row label="Tool version">{data.init?.tool_version ?? '-'}</Row>
            <Row label="Facts schema">{data.init?.facts_schema_version ?? '-'}</Row>
            <Row label="Last full snapshot">
              {data.last_snapshot?.full
                ? `${shortSha(data.last_snapshot.full.commit_sha)} · ${formatDateTime(data.last_snapshot.full.created_at)}`
                : '-'}
            </Row>
            <Row label="Last delta snapshot">
              {data.last_snapshot?.delta
                ? `${shortSha(data.last_snapshot.delta.commit_sha)} · ${formatDateTime(data.last_snapshot.delta.created_at)}`
                : '-'}
            </Row>
            <Row label="Active job">
              {data.active_job ? `${data.active_job.id} (${data.active_job.state})` : '-'}
            </Row>
            <Row label="Config hash">{data.config.hash ?? '-'}</Row>
            <Row label="Profile computed">{formatDateTime(data.profile_computed_at)}</Row>
          </dl>
        )}
        {data && data.config.validation_errors.length > 0 && (
          <div role="alert" className="rounded-md border border-destructive/30 p-3">
            <p className="font-medium text-destructive">Config validation errors</p>
            <ul className="mt-1 list-disc pl-5">
              {data.config.validation_errors.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          </div>
        )}
        {maintainer && (
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={busy} onClick={() => initialize.mutate()}>
              {initialize.isPending ? 'Initializing…' : 'Initialize'}
            </Button>
            <Button size="sm" variant="outline" disabled={busy} onClick={() => rebuild.mutate()}>
              {rebuild.isPending ? 'Requesting…' : 'Rebuild graph'}
            </Button>
          </div>
        )}
        {maintainer && actionError && (
          <p role="alert" className="text-destructive">
            {indexActionError(actionError)}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
