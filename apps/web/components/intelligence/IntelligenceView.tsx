'use client';

import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import { formatDateTime, formatPercent } from '@/lib/format';
import { repositoryIntelligenceQuery } from '@/lib/queries';
import { GraphStats } from './GraphStats';
import { SnapshotList } from './SnapshotList';

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card aria-label={title}>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

/** Repository Intelligence: snapshots, fingerprint and versions, graph health, index jobs. */
export function IntelligenceView({ repoId }: { repoId: string }) {
  const query = useQuery(repositoryIntelligenceQuery(repoId));
  if (query.isPending) return <Loading label="Loading intelligence" className="h-64" />;
  if (query.isError) {
    return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  }
  const intel = query.data;

  return (
    <div className="space-y-6">
      <div className="grid gap-6 lg:grid-cols-2">
        <Section title="Fingerprint and versions">
          <dl className="grid grid-cols-[10rem_1fr] gap-y-1 text-sm">
            <dt className="text-muted-foreground">Fingerprint</dt>
            <dd className="font-mono text-xs break-all">{intel.fingerprint ?? '-'}</dd>
            <dt className="text-muted-foreground">Tool version</dt>
            <dd>{intel.versions.tool_version ?? '-'}</dd>
            <dt className="text-muted-foreground">Facts schema</dt>
            <dd>{intel.versions.facts_schema_version ?? '-'}</dd>
            {Object.entries(intel.versions.analyzers).map(([name, version]) => (
              <div key={name} className="contents">
                <dt className="text-muted-foreground">{name}</dt>
                <dd>{version}</dd>
              </div>
            ))}
          </dl>
        </Section>
        <Section title="Languages and frameworks">
          <ul className="space-y-1 text-sm">
            {intel.languages.map((l) => (
              <li key={l.name} className="flex justify-between">
                <span>{l.name}</span>
                <span className="text-muted-foreground tabular-nums">
                  {l.files} files · {formatPercent(l.share)}
                </span>
              </li>
            ))}
          </ul>
          <div className="mt-3 flex flex-wrap gap-1">
            {intel.frameworks.map((f) => (
              <Badge key={f} variant="secondary">
                {f}
              </Badge>
            ))}
          </div>
        </Section>
      </div>

      <Section title="Graph">
        {intel.stats ? (
          <GraphStats stats={intel.stats} />
        ) : (
          <EmptyState title="No graph yet.">
            Graph statistics appear after the first index.
          </EmptyState>
        )}
      </Section>

      <Section title="Snapshots">
        <SnapshotList snapshots={intel.snapshots} />
      </Section>

      <Section title="Index jobs">
        {intel.index_jobs.length === 0 ? (
          <p className="text-sm text-muted-foreground">No index jobs yet.</p>
        ) : (
          <table className="w-full text-left text-sm" aria-label="Index jobs">
            <tbody>
              {intel.index_jobs.map((job) => (
                <tr key={job.id} className="border-t">
                  <td className="py-2 pr-4 font-mono text-xs">{job.id}</td>
                  <td className="py-2 pr-4 capitalize">{job.kind}</td>
                  <td className="py-2 pr-4">
                    <Badge
                      variant={
                        job.state === 'succeeded'
                          ? 'success'
                          : job.state === 'failed'
                            ? 'danger'
                            : job.state === 'cancelled'
                              ? 'muted'
                              : 'info'
                      }
                    >
                      {job.state}
                    </Badge>
                    {job.error_class && (
                      <span className="ml-2 text-xs text-muted-foreground">{job.error_class}</span>
                    )}
                  </td>
                  <td className="py-2 pr-4 text-muted-foreground">
                    {formatDateTime(job.created_at)}
                  </td>
                  <td className="py-2 text-muted-foreground">{formatDateTime(job.finished_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Section>
    </div>
  );
}
