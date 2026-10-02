'use client';

import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { dashboardQuery, type DashboardSummary } from '@/lib/dashboard';
import { SEVERITY_ORDER, SEVERITY_TOKENS } from '@/lib/severity';
import { ActiveRunsTable } from './ActiveRunsTable';
import { StatCard, type CardState } from './StatCard';

const pct = (fraction: number) => `${(fraction * 100).toFixed(1)}%`;
const seconds = (ms: number) => `${(ms / 1000).toFixed(1)}s`;

function Metric({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <p className="text-muted-foreground">{label}</p>
      <p className="text-xl font-semibold tabular-nums">{children}</p>
    </div>
  );
}

export function Dashboard({ orgId }: { orgId: string }) {
  const query = useQuery(dashboardQuery(orgId));
  const data: DashboardSummary | undefined = query.data;
  const base: CardState = query.isPending ? 'loading' : query.isError ? 'error' : 'ready';
  // A section the API could not compute degrades just that card.
  const stateOf = (section: unknown): CardState =>
    base === 'ready' && section == null ? 'unavailable' : base;
  const retry = () => void query.refetch();
  const windowDays = data?.window_days ?? 7;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Dashboard</h1>
        <p className="text-sm text-muted-foreground">
          Review activity over the last {windowDays} days.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard
          title="Reviews"
          href="/pull-requests"
          state={stateOf(data?.reviews)}
          onRetry={retry}
        >
          {data?.reviews && (
            <dl className="grid grid-cols-3 gap-2 text-sm">
              {(['completed', 'degraded', 'failed'] as const).map((key) => (
                <div key={key}>
                  <dt className="text-muted-foreground capitalize">{key}</dt>
                  <dd className="text-xl font-semibold tabular-nums">{data.reviews?.[key]}</dd>
                </div>
              ))}
            </dl>
          )}
        </StatCard>

        <StatCard
          title="Review latency"
          href="/pull-requests?sort=duration"
          state={stateOf(data?.latency_ms)}
          onRetry={retry}
        >
          {data?.latency_ms && (
            <div className="grid grid-cols-2 gap-2 text-sm">
              <Metric label="Median">{seconds(data.latency_ms.median)}</Metric>
              <Metric label="p95">{seconds(data.latency_ms.p95)}</Metric>
            </div>
          )}
        </StatCard>

        <StatCard
          title="Published findings"
          href="/pull-requests?view=findings"
          state={stateOf(data?.findings_by_severity)}
          onRetry={retry}
        >
          {data?.findings_by_severity && (
            <ul className="space-y-1 text-sm">
              {SEVERITY_ORDER.map((severity) => (
                <li key={severity} className="flex items-center justify-between">
                  <span className={`flex items-center gap-2 ${SEVERITY_TOKENS[severity].text}`}>
                    <span
                      aria-hidden
                      className={`size-2 rounded-full ${SEVERITY_TOKENS[severity].dot}`}
                    />
                    {SEVERITY_TOKENS[severity].label}
                  </span>
                  <span className="tabular-nums">{data.findings_by_severity?.[severity]}</span>
                </li>
              ))}
            </ul>
          )}
        </StatCard>

        <StatCard
          title="Finding quality"
          href="/pull-requests?view=feedback"
          state={stateOf(data?.quality)}
          onRetry={retry}
        >
          {data?.quality && (
            <div className="grid grid-cols-2 gap-2 text-sm">
              <Metric label="Acceptance">{pct(data.quality.acceptance_rate)}</Metric>
              <Metric label="False positives">{pct(data.quality.false_positive_rate)}</Metric>
            </div>
          )}
        </StatCard>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Active runs</CardTitle>
        </CardHeader>
        <CardContent>
          {base === 'loading' && (
            <div
              role="status"
              aria-label="Loading active runs"
              className="h-16 animate-pulse rounded bg-muted"
            />
          )}
          {base === 'error' && (
            <p role="alert" className="text-sm text-destructive">
              Could not load active runs.
            </p>
          )}
          {base === 'ready' && data && <ActiveRunsTable runs={data.active_runs} />}
        </CardContent>
      </Card>
    </div>
  );
}
