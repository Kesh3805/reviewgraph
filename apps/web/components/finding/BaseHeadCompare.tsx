'use client';

import { useQuery } from '@tanstack/react-query';
import { Badge } from '@/components/ui/badge';
import { ErrorState, Loading } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import type { AnchorSide } from '@/lib/api/pending';
import { sourceExcerptQuery } from '@/lib/queries';
import { SourceExcerpt } from './SourceExcerpt';

function PredicateBadge({ predicate }: { predicate: AnchorSide['predicate'] }) {
  if (predicate.holds === null) {
    return <Badge variant="muted">{predicate.name}: not evaluated</Badge>;
  }
  return (
    <Badge variant={predicate.holds ? 'danger' : 'success'}>
      {predicate.name}: {predicate.holds ? 'holds' : 'does not hold'}
    </Badge>
  );
}

function Side({
  repoId,
  label,
  side,
}: {
  repoId: string;
  label: 'Base' | 'Head';
  side: AnchorSide | null;
}) {
  const query = useQuery({
    ...sourceExcerptQuery(repoId, {
      path: side?.path ?? '',
      start: side?.start_line ?? 0,
      end: side?.end_line ?? 0,
      snapshot: side?.snapshot_id,
    }),
    enabled: !!side,
  });
  const missingAtBase =
    label === 'Base' && (!side || (query.error instanceof ApiError && query.error.status === 404));

  return (
    <div className="min-w-0 space-y-2" data-testid={`side-${label.toLowerCase()}`}>
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-medium">{label}</h3>
        {side && <PredicateBadge predicate={side.predicate} />}
      </div>
      {missingAtBase ? (
        <p className="rounded-md border border-dashed p-4 text-sm text-muted-foreground">
          The file did not exist at base.
        </p>
      ) : query.isPending ? (
        <Loading label={`Loading ${label.toLowerCase()} excerpt`} />
      ) : query.isError ? (
        <ErrorState error={query.error} onRetry={() => void query.refetch()} />
      ) : (
        <SourceExcerpt excerpt={query.data} />
      )}
    </div>
  );
}

/** Side-by-side excerpts of the anchor symbol at base and head, with the predicate result. */
export function BaseHeadCompare({
  repoId,
  base,
  head,
}: {
  repoId: string;
  base: AnchorSide | null;
  head: AnchorSide;
}) {
  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <Side repoId={repoId} label="Base" side={base} />
      <Side repoId={repoId} label="Head" side={head} />
    </div>
  );
}
