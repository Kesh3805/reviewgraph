'use client';

import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { SeverityLabel } from '@/components/review/FindingCard';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { HighlightedCode } from '@/components/ui/code';
import { ErrorState, Loading } from '@/components/ui/states';
import { repositoryRulesQuery, repositoryStatusQuery } from '@/lib/queries';
import { canMaintain } from '@/lib/roles';
import { SuppressionsPanel } from './SuppressionsPanel';

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

/**
 * Rules: the effective `.review/config.yaml` (read-only; it is repository-owned), its validation
 * errors, explicit rules with recent violations, and suppressions.
 */
export function RulesView({ repoId }: { repoId: string }) {
  const org = useCurrentOrg();
  const maintainer = canMaintain(org?.role);
  const rules = useQuery(repositoryRulesQuery(repoId));
  const status = useQuery(repositoryStatusQuery(repoId));
  const validationErrors = status.data?.config.validation_errors ?? [];

  return (
    <div className="space-y-6">
      <Section title="Configuration">
        {validationErrors.length > 0 && (
          <div role="alert" className="mb-3 rounded-md border border-destructive/30 p-3 text-sm">
            <p className="font-medium text-destructive">
              {validationErrors.length} validation error{validationErrors.length > 1 ? 's' : ''}
            </p>
            <ul className="mt-1 list-disc pl-5">
              {validationErrors.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          </div>
        )}
        {rules.isPending && <Loading label="Loading configuration" />}
        {rules.isError && <ErrorState error={rules.error} onRetry={() => void rules.refetch()} />}
        {rules.data &&
          (rules.data.config.yaml ? (
            <div className="overflow-x-auto rounded-md border bg-muted/30">
              <p className="border-b px-3 py-1 font-mono text-xs text-muted-foreground">
                {rules.data.config.path} · config {status.data?.config.hash?.slice(0, 12) ?? '-'}
              </p>
              <HighlightedCode text={rules.data.config.yaml} language="yaml" testId="config-yaml" />
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              No {rules.data.config.path} in the repository: defaults apply.
            </p>
          ))}
      </Section>

      <Section title="Explicit rules">
        {rules.data && rules.data.rules.length === 0 && (
          <p className="text-sm text-muted-foreground">No explicit rules.</p>
        )}
        {rules.data && rules.data.rules.length > 0 && (
          <table className="w-full text-left text-sm" aria-label="Explicit rules">
            <thead className="text-xs text-muted-foreground uppercase">
              <tr>
                <th className="py-2 pr-4 font-medium">Rule</th>
                <th className="py-2 pr-4 font-medium">Severity</th>
                <th className="py-2 text-right font-medium">Violations (30 days)</th>
              </tr>
            </thead>
            <tbody>
              {rules.data.rules.map((r) => (
                <tr key={r.id} className="border-t">
                  <td className="py-2 pr-4">
                    <span className="font-mono text-xs">{r.id}</span>
                    <p className="text-muted-foreground">{r.description}</p>
                  </td>
                  <td className="py-2 pr-4">
                    <SeverityLabel severity={r.severity} />
                  </td>
                  <td className="py-2 text-right tabular-nums">{r.violations_30d}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Section>

      <Section title="Suppressions">
        <SuppressionsPanel repoId={repoId} maintainer={maintainer} />
      </Section>
    </div>
  );
}
