import { Badge, type BadgeVariant } from '@/components/ui/badge';
import type { IndexState, RepositoryStatus } from '@/lib/api/pending';

const INDEX_STATE: Record<IndexState, { label: string; variant: BadgeVariant }> = {
  not_initialized: { label: 'Not initialized', variant: 'muted' },
  queued: { label: 'Queued', variant: 'info' },
  indexing: { label: 'Indexing', variant: 'info' },
  ready: { label: 'Indexed', variant: 'success' },
};

export function IndexStateBadge({ state }: { state: IndexState }) {
  const { label, variant } = INDEX_STATE[state];
  return <Badge variant={variant}>{label}</Badge>;
}

/** The newest of the latest full and delta snapshots, else the last init. */
export function lastIndexedAt(status: RepositoryStatus): string | null {
  const times = [
    status.last_snapshot?.full?.created_at,
    status.last_snapshot?.delta?.created_at,
  ].filter((t): t is string => !!t);
  if (times.length > 0) return times.sort().at(-1) ?? null;
  return status.init?.detected_at ?? null;
}
