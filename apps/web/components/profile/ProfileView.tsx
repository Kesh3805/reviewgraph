'use client';

import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { EmptyState, ErrorState, Loading } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { formatDateTime } from '@/lib/format';
import { repositoryProfileQuery } from '@/lib/queries';
import { ConventionsTable } from './ConventionsTable';
import { EffectivePolicy } from './EffectivePolicy';
import { LayerMatrix } from './LayerMatrix';

/** Where the configuration reference explains overriding the inferred profile. */
export const OVERRIDE_DOCS_HREF =
  'https://github.com/Kesh3805/reviewgraph/blob/main/docs/architecture/target-architecture.md';

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

/** Repository Profile: layers, module matrix, conventions, documentation rules, policy. */
export function ProfileView({ repoId }: { repoId: string }) {
  const query = useQuery(repositoryProfileQuery(repoId));
  if (query.isPending) return <Loading label="Loading profile" className="h-64" />;
  if (query.isError) {
    if (query.error instanceof ApiError && query.error.status === 404) {
      return (
        <EmptyState title="No profile yet.">
          Profile is computed after the first full index.
        </EmptyState>
      );
    }
    return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  }
  const profile = query.data;

  return (
    <div className="space-y-6">
      <p className="text-sm text-muted-foreground">
        Profile v{profile.profile_version} · computed {formatDateTime(profile.computed_at)} · config{' '}
        {profile.config_hash.slice(0, 12)} ·{' '}
        <a
          href={OVERRIDE_DOCS_HREF}
          className="underline"
          target="_blank"
          rel="noreferrer noopener"
        >
          How to override
        </a>
      </p>
      <Section title="Layers">
        <LayerMatrix architecture={profile.architecture} />
      </Section>
      <Section title="Conventions">
        <ConventionsTable conventions={profile.conventions} />
      </Section>
      <div className="grid gap-6 lg:grid-cols-2">
        <Section title="Documentation rules">
          {profile.documentation_rules.length === 0 ? (
            <p className="text-sm text-muted-foreground">No documentation rules.</p>
          ) : (
            <ul className="space-y-1 text-sm">
              {profile.documentation_rules.map((d) => (
                <li key={d.id}>
                  <span className="font-medium">{d.title}</span>{' '}
                  <span className="font-mono text-xs text-muted-foreground">{d.path}</span>
                  {d.topics.length > 0 && (
                    <span className="text-xs text-muted-foreground"> · {d.topics.join(', ')}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </Section>
        <Section title="Effective policy">
          <EffectivePolicy policies={profile.effective_policy ?? []} />
        </Section>
      </div>
    </div>
  );
}
