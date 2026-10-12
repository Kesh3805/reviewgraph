'use client';

import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Input, Label, Select } from '@/components/ui/input';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import type { UsageGroupBy, UsageReport } from '@/lib/api/pending';
import { formatNumber, formatUsdMicros } from '@/lib/format';
import { usageQuery } from '@/lib/queries';

const GROUPS: { value: UsageGroupBy; label: string }[] = [
  { value: 'day', label: 'Day' },
  { value: 'reviewer', label: 'Reviewer' },
  { value: 'tier', label: 'Model tier' },
  { value: 'model', label: 'Model' },
];

/** `YYYY-MM-DD` for `daysAgo` days before `now` (UTC). */
export function isoDay(daysAgo = 0, now = Date.now()): string {
  return new Date(now - daysAgo * 86_400_000).toISOString().slice(0, 10);
}

function Totals({ report }: { report: UsageReport }) {
  const t = report.totals;
  const items: [string, string, string][] = [
    ['Cost', formatUsdMicros(t.cost_usd_micros), 'usage-cost'],
    ['Model calls', formatNumber(t.model_calls), 'usage-calls'],
    [
      'Tokens in / out',
      `${formatNumber(t.input_tokens)} / ${formatNumber(t.output_tokens)}`,
      'usage-tokens',
    ],
    [
      'Cost per reviewed PR',
      formatUsdMicros(report.cost_per_reviewed_pr_usd_micros),
      'usage-per-pr',
    ],
    [
      'Cost per useful finding',
      formatUsdMicros(report.cost_per_useful_finding_usd_micros),
      'usage-per-finding',
    ],
  ];
  return (
    <dl className="grid gap-4 sm:grid-cols-3 lg:grid-cols-5">
      {items.map(([label, value, id]) => (
        <Card key={id} className="gap-2 py-4">
          <CardContent className="px-4">
            <dt className="text-xs text-muted-foreground">{label}</dt>
            <dd className="text-lg font-semibold tabular-nums" data-testid={id}>
              {value}
            </dd>
          </CardContent>
        </Card>
      ))}
    </dl>
  );
}

/** Tokens, model calls and cost by day, reviewer, model tier or model (PRD §115, QB-005). */
export function UsageView() {
  const org = useCurrentOrg();
  const [groupBy, setGroupBy] = useState<UsageGroupBy>('day');
  const [from, setFrom] = useState(() => isoDay(30));
  const [to, setTo] = useState(() => isoDay(0));
  const query = useQuery({
    ...usageQuery(org?.id ?? '', { from, to, group_by: groupBy }),
    enabled: !!org,
  });
  const groupLabel = GROUPS.find((g) => g.value === groupBy)?.label ?? groupBy;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Usage</h1>
        <p className="text-sm text-muted-foreground">Model usage and cost for this organization.</p>
      </div>
      {!org && <EmptyState title="You are not a member of any organization yet." />}
      <div className="flex flex-wrap items-end gap-3">
        <div className="space-y-1">
          <Label htmlFor="usage-from">From</Label>
          <Input
            id="usage-from"
            type="date"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="usage-to">To</Label>
          <Input id="usage-to" type="date" value={to} onChange={(e) => setTo(e.target.value)} />
        </div>
        <div className="space-y-1">
          <Label htmlFor="usage-group">Group by</Label>
          <Select
            id="usage-group"
            className="w-40"
            value={groupBy}
            onChange={(e) => setGroupBy(e.target.value as UsageGroupBy)}
          >
            {GROUPS.map((g) => (
              <option key={g.value} value={g.value}>
                {g.label}
              </option>
            ))}
          </Select>
        </div>
      </div>

      {org && query.isPending && <Loading label="Loading usage" />}
      {query.isError && <ErrorState error={query.error} onRetry={() => void query.refetch()} />}
      {query.data && (
        <>
          <Totals report={query.data} />
          <Card>
            <CardHeader>
              <CardTitle>By {groupLabel.toLowerCase()}</CardTitle>
            </CardHeader>
            <CardContent>
              {query.data.rows.length === 0 ? (
                <p className="text-sm text-muted-foreground">No model calls in this range.</p>
              ) : (
                <table className="w-full text-left text-sm" aria-label="Usage">
                  <thead className="text-xs text-muted-foreground uppercase">
                    <tr>
                      <th className="py-2 pr-4 font-medium">{groupLabel}</th>
                      <th className="py-2 pr-4 text-right font-medium">Calls</th>
                      <th className="py-2 pr-4 text-right font-medium">Input</th>
                      <th className="py-2 pr-4 text-right font-medium">Output</th>
                      <th className="py-2 pr-4 text-right font-medium">Cached</th>
                      <th className="py-2 text-right font-medium">Cost</th>
                    </tr>
                  </thead>
                  <tbody>
                    {query.data.rows.map((r) => (
                      <tr key={r.key} className="border-t">
                        <td className="py-2 pr-4 font-mono text-xs">{r.key}</td>
                        <td className="py-2 pr-4 text-right tabular-nums">
                          {formatNumber(r.model_calls)}
                        </td>
                        <td className="py-2 pr-4 text-right tabular-nums">
                          {formatNumber(r.input_tokens)}
                        </td>
                        <td className="py-2 pr-4 text-right tabular-nums">
                          {formatNumber(r.output_tokens)}
                        </td>
                        <td className="py-2 pr-4 text-right tabular-nums">
                          {formatNumber(r.cached_tokens)}
                        </td>
                        <td className="py-2 text-right tabular-nums">
                          {formatUsdMicros(r.cost_usd_micros)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
