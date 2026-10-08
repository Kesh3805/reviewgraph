'use client';

import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { Badge, type BadgeVariant } from '@/components/ui/badge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import type { GithubIntegration } from '@/lib/api/pending';
import { formatDateTime } from '@/lib/format';
import { githubIntegrationQuery } from '@/lib/queries';

const RECONCILER: Record<GithubIntegration['reconciler']['state'], BadgeVariant> = {
  ok: 'success',
  lagging: 'warning',
  failing: 'danger',
  disabled: 'muted',
};

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card aria-label={title}>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 text-sm">{children}</CardContent>
    </Card>
  );
}

/** GitHub App installation, permission check, webhook health and reconciler status. */
export function IntegrationsView() {
  const org = useCurrentOrg();
  const query = useQuery({ ...githubIntegrationQuery(org?.id ?? ''), enabled: !!org });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Integrations</h1>
        <p className="text-sm text-muted-foreground">GitHub App status for this organization.</p>
      </div>
      {!org && <EmptyState title="You are not a member of any organization yet." />}
      {org && query.isPending && <Loading label="Loading integration" />}
      {query.isError && <ErrorState error={query.error} onRetry={() => void query.refetch()} />}
      {query.data && <IntegrationCards data={query.data} />}
    </div>
  );
}

function IntegrationCards({ data }: { data: GithubIntegration }) {
  const { installation, permissions_check: perms, webhooks, reconciler } = data;
  return (
    <div className="grid gap-6 lg:grid-cols-2">
      <Section title="Installation">
        {installation ? (
          <>
            <p className="flex items-center gap-2">
              <span className="font-medium">{installation.account_login}</span>
              <Badge variant={installation.state === 'active' ? 'success' : 'danger'}>
                {installation.state}
              </Badge>
            </p>
            <p className="text-muted-foreground">
              Installed {formatDateTime(installation.installed_at)}
            </p>
          </>
        ) : (
          <p className="text-muted-foreground">The GitHub App is not installed.</p>
        )}
      </Section>

      <Section title="Permissions">
        <p className="flex items-center gap-2" data-testid="permission-check">
          {perms.ok ? (
            <Badge variant="success">Permissions OK</Badge>
          ) : (
            <Badge variant="danger">Permission check failed</Badge>
          )}
          <span className="text-muted-foreground">checked {formatDateTime(perms.checked_at)}</span>
        </p>
        {perms.missing.length > 0 && (
          <p>
            Missing: <span className="font-mono text-xs">{perms.missing.join(', ')}</span>
          </p>
        )}
        {perms.excess.length > 0 && (
          <p role="alert" className="text-destructive">
            Excess (must be removed):{' '}
            <span className="font-mono text-xs">{perms.excess.join(', ')}</span>
          </p>
        )}
      </Section>

      <Section title="Webhooks">
        <dl className="grid grid-cols-[12rem_1fr] gap-y-1">
          <dt className="text-muted-foreground">Last delivery</dt>
          <dd>
            {formatDateTime(webhooks.last_delivery_at)}
            {webhooks.last_delivery_event && ` · ${webhooks.last_delivery_event}`}
          </dd>
          <dt className="text-muted-foreground">Deliveries (24 h)</dt>
          <dd className="tabular-nums">{webhooks.deliveries_24h}</dd>
          <dt className="text-muted-foreground">Signature failures (24 h)</dt>
          <dd
            className={`tabular-nums ${webhooks.signature_failures_24h > 0 ? 'font-medium text-destructive' : ''}`}
          >
            {webhooks.signature_failures_24h}
          </dd>
        </dl>
      </Section>

      <Section title="Reconciler">
        <p className="flex items-center gap-2">
          <Badge variant={RECONCILER[reconciler.state]}>{reconciler.state}</Badge>
          <span className="text-muted-foreground">
            last run {formatDateTime(reconciler.last_run_at)}
          </span>
        </p>
        <p className="text-muted-foreground">
          {reconciler.repaired_24h} missed events repaired in the last 24 h
        </p>
      </Section>
    </div>
  );
}
