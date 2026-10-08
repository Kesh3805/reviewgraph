'use client';

import { useQuery } from '@tanstack/react-query';
import { Select } from '@/components/ui/input';
import { shortSha } from '@/lib/format';
import { repositoryIntelligenceQuery } from '@/lib/queries';

/**
 * Snapshot to query: the default branch's latest (empty value, resolved by the API), an indexed
 * snapshot, or a PR head snapshot deep-linked from a review (`?snapshot=`).
 */
export function SnapshotSelect({
  repoId,
  value,
  onChange,
}: {
  repoId: string;
  value: string | undefined;
  onChange: (snapshot: string | undefined) => void;
}) {
  const intel = useQuery(repositoryIntelligenceQuery(repoId));
  const snapshots = intel.data?.snapshots ?? [];
  const known = !value || snapshots.some((s) => s.id === value);

  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="text-muted-foreground">Snapshot</span>
      <Select
        aria-label="Snapshot"
        className="w-72"
        value={value ?? ''}
        onChange={(e) => onChange(e.target.value || undefined)}
      >
        <option value="">Default branch (latest)</option>
        {!known && value && <option value={value}>PR head {value.slice(0, 8)}</option>}
        {snapshots.map((s) => (
          <option key={s.id} value={s.id}>
            {s.kind} · {s.branch ?? 'detached'} · {shortSha(s.commit_sha)}
          </option>
        ))}
      </Select>
    </label>
  );
}
