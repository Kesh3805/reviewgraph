'use client';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, type FormEvent } from 'react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input, Label, Select } from '@/components/ui/input';
import { ErrorState, Loading, errorMessage } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { createSuppression, revokeSuppression } from '@/lib/api/endpoints';
import type { CreateSuppressionInput, Suppression } from '@/lib/api/pending';
import { formatDateTime } from '@/lib/format';
import { keys, suppressionsQuery } from '@/lib/queries';

function actionError(error: unknown): string {
  if (error instanceof ApiError && error.status === 403) return 'Requires maintainer role.';
  return errorMessage(error, 'The request failed.');
}

function CreateSuppressionForm({ repoId }: { repoId: string }) {
  const queryClient = useQueryClient();
  const [kind, setKind] = useState<Suppression['kind']>('path');
  const [value, setValue] = useState('');
  const [reason, setReason] = useState('');
  const create = useMutation({
    mutationFn: (input: CreateSuppressionInput) => createSuppression(repoId, input),
    onSuccess: async () => {
      setValue('');
      setReason('');
      await queryClient.invalidateQueries({ queryKey: keys.suppressions(repoId) });
    },
  });
  const valid = value.trim().length > 0 && reason.trim().length > 0;

  function submit(e: FormEvent) {
    e.preventDefault();
    if (valid) create.mutate({ kind, value: value.trim(), reason: reason.trim() });
  }

  return (
    <form
      onSubmit={submit}
      aria-label="Create suppression"
      className="flex flex-wrap items-end gap-2"
    >
      <div className="space-y-1">
        <Label htmlFor="suppression-kind">Kind</Label>
        <Select
          id="suppression-kind"
          className="w-36"
          value={kind}
          onChange={(e) => setKind(e.target.value as Suppression['kind'])}
        >
          <option value="path">Path</option>
          <option value="symbol">Symbol</option>
          <option value="fingerprint">Fingerprint</option>
          <option value="rule">Rule</option>
        </Select>
      </div>
      <div className="min-w-48 flex-1 space-y-1">
        <Label htmlFor="suppression-value">Value</Label>
        <Input
          id="suppression-value"
          placeholder="src/generated/**"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
      </div>
      <div className="min-w-48 flex-1 space-y-1">
        <Label htmlFor="suppression-reason">Reason</Label>
        <Input id="suppression-reason" value={reason} onChange={(e) => setReason(e.target.value)} />
      </div>
      <Button type="submit" disabled={!valid || create.isPending}>
        {create.isPending ? 'Creating…' : 'Create suppression'}
      </Button>
      {create.isError && (
        <p role="alert" className="w-full text-sm text-destructive">
          {actionError(create.error)}
        </p>
      )}
    </form>
  );
}

/** Suppressions with create/revoke (maintainers) and the audit history. */
export function SuppressionsPanel({ repoId, maintainer }: { repoId: string; maintainer: boolean }) {
  const queryClient = useQueryClient();
  const query = useQuery(suppressionsQuery(repoId));
  const revoke = useMutation({
    mutationFn: (id: string) => revokeSuppression(repoId, id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: keys.suppressions(repoId) }),
  });

  if (query.isPending) return <Loading label="Loading suppressions" />;
  if (query.isError) return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  const { items, audit } = query.data;

  return (
    <div className="space-y-4">
      {maintainer && <CreateSuppressionForm repoId={repoId} />}
      {items.length === 0 ? (
        <p className="text-sm text-muted-foreground">No suppressions.</p>
      ) : (
        <table className="w-full text-left text-sm" aria-label="Suppressions">
          <tbody>
            {items.map((s) => (
              <tr key={s.id} className="border-t align-top" data-testid={`suppression-${s.id}`}>
                <td className="py-2 pr-3">
                  <Badge variant="outline">{s.kind}</Badge>
                </td>
                <td className="py-2 pr-3 font-mono text-xs break-all">{s.value}</td>
                <td className="py-2 pr-3">{s.reason}</td>
                <td className="py-2 pr-3 text-xs text-muted-foreground">
                  {s.created_by} · {formatDateTime(s.created_at)}
                </td>
                <td className="py-2 text-right">
                  {s.revoked_at ? (
                    <Badge variant="muted">Revoked</Badge>
                  ) : (
                    maintainer && (
                      <Button
                        size="sm"
                        variant="outline"
                        disabled={revoke.isPending}
                        onClick={() => revoke.mutate(s.id)}
                        aria-label={`Revoke suppression ${s.value}`}
                      >
                        Revoke
                      </Button>
                    )
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {revoke.isError && (
        <p role="alert" className="text-sm text-destructive">
          {actionError(revoke.error)}
        </p>
      )}
      <div>
        <h3 className="mb-1 text-xs font-medium text-muted-foreground uppercase">Audit history</h3>
        {audit.length === 0 ? (
          <p className="text-sm text-muted-foreground">No changes recorded.</p>
        ) : (
          <ul aria-label="Audit history" className="space-y-1 text-sm">
            {audit.map((a) => (
              <li key={a.id}>
                <span className="text-muted-foreground">{formatDateTime(a.at)}</span> {a.actor}{' '}
                <span className="font-medium">{a.action}</span>{' '}
                <span className="font-mono text-xs">{a.target}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
