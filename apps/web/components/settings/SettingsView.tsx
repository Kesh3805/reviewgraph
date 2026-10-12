'use client';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent, type ReactNode } from 'react';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Input, Label, Select } from '@/components/ui/input';
import { EmptyState, ErrorState, Loading, errorMessage } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { updateMemberRole, updateOrganizationSettings } from '@/lib/api/endpoints';
import type { Member, OrganizationSettings } from '@/lib/api/pending';
import { formatDateTime } from '@/lib/format';
import { keys, membersQuery, orgSettingsQuery } from '@/lib/queries';
import { isAdmin } from '@/lib/roles';

export const CONFLICT_MESSAGE =
  'These settings were changed by someone else. Reload to see the latest values.';

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card aria-label={title}>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">{children}</CardContent>
    </Card>
  );
}

function saveError(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 409) return CONFLICT_MESSAGE;
    if (error.status === 403) return 'Requires the admin role.';
  }
  return errorMessage(error, 'Could not save the settings.');
}

function ExternalModelsBanner({ enabled }: { enabled: boolean }) {
  if (enabled) return null;
  return (
    <div
      role="status"
      className="rounded-md border border-amber-500/50 bg-amber-50 p-3 text-sm dark:bg-amber-950/30"
    >
      <p className="font-medium">External models are disabled.</p>
      <p className="text-muted-foreground">
        Reviews started from now on send no source to external model providers.
      </p>
    </div>
  );
}

function SettingsForm({
  orgId,
  settings,
  admin,
}: {
  orgId: string;
  settings: OrganizationSettings;
  admin: boolean;
}) {
  const queryClient = useQueryClient();
  const [sourceDays, setSourceDays] = useState(String(settings.retention.source_days));
  const [artifactDays, setArtifactDays] = useState(String(settings.retention.artifacts_days));
  const [external, setExternal] = useState(settings.external_models);
  const save = useMutation({
    mutationFn: () =>
      updateOrganizationSettings(orgId, {
        retention: { source_days: Number(sourceDays), artifacts_days: Number(artifactDays) },
        external_models: external,
        // Optimistic concurrency: the API answers 409 if someone saved in between.
        updated_at: settings.updated_at,
      }),
    onSuccess: (saved) => queryClient.setQueryData(keys.orgSettings(orgId), saved),
  });
  const valid = [sourceDays, artifactDays].every((d) => /^\d+$/.test(d) && Number(d) >= 1);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (valid) save.mutate();
  }

  return (
    <form onSubmit={submit} aria-label="Organization settings" className="space-y-4">
      <ExternalModelsBanner enabled={settings.external_models} />
      <fieldset disabled={!admin || save.isPending} className="space-y-4">
        <div className="flex flex-wrap gap-4">
          <div className="space-y-1">
            <Label htmlFor="retention-source">Source retention (days)</Label>
            <Input
              id="retention-source"
              inputMode="numeric"
              className="w-32"
              value={sourceDays}
              onChange={(e) => setSourceDays(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="retention-artifacts">Artifact retention (days)</Label>
            <Input
              id="retention-artifacts"
              inputMode="numeric"
              className="w-32"
              value={artifactDays}
              onChange={(e) => setArtifactDays(e.target.value)}
            />
          </div>
        </div>
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            className="size-4"
            checked={external}
            onChange={(e) => setExternal(e.target.checked)}
          />
          Allow external model providers
        </label>
      </fieldset>
      {admin ? (
        <div className="flex flex-wrap items-center gap-3">
          <Button type="submit" disabled={!valid || save.isPending}>
            {save.isPending ? 'Saving…' : 'Save settings'}
          </Button>
          {!valid && <span className="text-destructive">Retention must be at least one day.</span>}
          {save.isSuccess && <span role="status">Saved.</span>}
          {save.isError && (
            <span role="alert" className="text-destructive">
              {saveError(save.error)}
              {save.error instanceof ApiError && save.error.status === 409 && (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="ml-2"
                  onClick={() =>
                    void queryClient.invalidateQueries({ queryKey: keys.orgSettings(orgId) })
                  }
                >
                  Reload
                </Button>
              )}
            </span>
          )}
        </div>
      ) : (
        <p className="text-muted-foreground">Only organization admins can change these settings.</p>
      )}
      <p className="text-xs text-muted-foreground">
        Last updated {formatDateTime(settings.updated_at)}
      </p>
    </form>
  );
}

const ROLES: Member['role'][] = ['owner', 'admin', 'member', 'viewer'];

function MembersTable({ orgId, admin }: { orgId: string; admin: boolean }) {
  const queryClient = useQueryClient();
  const members = useQuery(membersQuery(orgId));
  const change = useMutation({
    mutationFn: ({ userId, role }: { userId: string; role: Member['role'] }) =>
      updateMemberRole(orgId, userId, role),
    onSettled: () => queryClient.invalidateQueries({ queryKey: keys.members(orgId) }),
  });
  if (members.isPending) return <Loading label="Loading members" />;
  if (members.isError)
    return <ErrorState error={members.error} onRetry={() => void members.refetch()} />;
  return (
    <>
      <table className="w-full text-left" aria-label="Members">
        <tbody>
          {members.data.map((m) => (
            <tr key={m.user_id} className="border-t">
              <td className="py-2 pr-4">
                {m.display_name ?? m.login}{' '}
                <span className="text-muted-foreground">@{m.login}</span>
              </td>
              <td className="py-2 text-right">
                {admin ? (
                  <Select
                    aria-label={`Role of ${m.login}`}
                    className="ml-auto w-32"
                    value={m.role}
                    disabled={change.isPending}
                    onChange={(e) =>
                      change.mutate({ userId: m.user_id, role: e.target.value as Member['role'] })
                    }
                  >
                    {ROLES.map((r) => (
                      <option key={r} value={r}>
                        {r}
                      </option>
                    ))}
                  </Select>
                ) : (
                  <Badge variant="outline">{m.role}</Badge>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {change.isError && (
        <p role="alert" className="text-destructive">
          {saveError(change.error)}
        </p>
      )}
    </>
  );
}

/** Members and roles, data retention and model privacy. Changes require admin (and are audited). */
export function SettingsView() {
  const org = useCurrentOrg();
  const admin = isAdmin(org?.role);
  const settings = useQuery({ ...orgSettingsQuery(org?.id ?? ''), enabled: !!org });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="text-sm text-muted-foreground">
          {org ? org.displayName : 'Organization'} settings. Every change is recorded in the audit
          log.
        </p>
      </div>
      {!org && <EmptyState title="You are not a member of any organization yet." />}
      {org && (
        <>
          <Section title="Data and model privacy">
            {settings.isPending && <Loading label="Loading settings" />}
            {settings.isError && (
              <ErrorState error={settings.error} onRetry={() => void settings.refetch()} />
            )}
            {settings.data && (
              <SettingsForm
                key={settings.data.updated_at}
                orgId={org.id}
                settings={settings.data}
                admin={admin}
              />
            )}
          </Section>
          <Section title="Members">
            <MembersTable orgId={org.id} admin={admin} />
          </Section>
        </>
      )}
    </div>
  );
}
