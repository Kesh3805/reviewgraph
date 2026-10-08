'use client';

import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { RunStateBadge } from '@/components/pulls/RunStateBadge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import { formatDateTime, formatPercent } from '@/lib/format';
import { pullRequestsQuery, repositoryQuery, riskAreasQuery } from '@/lib/queries';
import { canMaintain } from '@/lib/roles';
import { SEVERITY_TOKENS } from '@/lib/severity';
import { RepoSettingsForm } from './RepoSettingsForm';
import { StatusCard } from './StatusCard';

function RecentReviews({ repoId }: { repoId: string }) {
  const pulls = useQuery(pullRequestsQuery({ repository_id: repoId, limit: 5 }));
  if (pulls.isPending) return <Loading label="Loading recent reviews" className="h-16" />;
  if (pulls.isError) return <ErrorState error={pulls.error} onRetry={() => void pulls.refetch()} />;
  const reviewed = pulls.data.items.flatMap((pr) =>
    pr.latest_run ? [{ pr, run: pr.latest_run }] : [],
  );
  if (reviewed.length === 0) return <EmptyState title="No reviews yet." />;
  return (
    <ul className="divide-y text-sm">
      {reviewed.map(({ pr, run }) => (
        <li key={pr.id} className="flex flex-wrap items-center justify-between gap-2 py-2">
          <Link href={`/reviews/${run.id}`} className="min-w-0 truncate hover:underline">
            #{pr.number} {pr.title}
          </Link>
          <span className="flex items-center gap-2 text-muted-foreground">
            <RunStateBadge state={run.state} degraded={run.degraded} />
            {formatDateTime(run.created_at)}
          </span>
        </li>
      ))}
    </ul>
  );
}

function TopRiskAreas({ repoId }: { repoId: string }) {
  const areas = useQuery(riskAreasQuery(repoId));
  if (areas.isPending) return <Loading label="Loading risk areas" className="h-16" />;
  if (areas.isError) return <ErrorState error={areas.error} onRetry={() => void areas.refetch()} />;
  if (areas.data.length === 0) return <EmptyState title="No risk areas identified yet." />;
  return (
    <ul className="divide-y text-sm">
      {areas.data.map((area) => (
        <li key={area.path} className="flex items-center justify-between gap-2 py-2">
          <span className="min-w-0 truncate font-mono text-xs">{area.path}</span>
          <span className="flex shrink-0 items-center gap-3">
            <span className={SEVERITY_TOKENS[area.top_severity].text}>
              {SEVERITY_TOKENS[area.top_severity].label}
            </span>
            <span className="text-muted-foreground tabular-nums">{area.findings} findings</span>
            <span className="tabular-nums">{formatPercent(area.score)}</span>
          </span>
        </li>
      ))}
    </ul>
  );
}

export function RepoOverview({ repoId }: { repoId: string }) {
  const org = useCurrentOrg();
  const maintainer = canMaintain(org?.role);
  const repo = useQuery(repositoryQuery(repoId));

  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <StatusCard repoId={repoId} maintainer={maintainer} />
      <Card>
        <CardHeader>
          <CardTitle>Recent reviews</CardTitle>
        </CardHeader>
        <CardContent>
          <RecentReviews repoId={repoId} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Top risk areas</CardTitle>
        </CardHeader>
        <CardContent>
          <TopRiskAreas repoId={repoId} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Settings</CardTitle>
        </CardHeader>
        <CardContent>
          {repo.isPending && <Loading label="Loading settings" />}
          {repo.isError && <ErrorState error={repo.error} onRetry={() => void repo.refetch()} />}
          {repo.data && (
            <RepoSettingsForm
              key={repo.data.updated_at}
              repository={repo.data}
              maintainer={maintainer}
            />
          )}
        </CardContent>
      </Card>
    </div>
  );
}
