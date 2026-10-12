'use client';

import { useQuery } from '@tanstack/react-query';
import { ErrorState, Loading } from '@/components/ui/states';
import { getReviewImpact } from '@/lib/api/endpoints';
import type { ImpactGraph } from '@/lib/api/pending';
import { PathTabs } from './PathTabs';

/** The ImpactGraph categories as grouped lists. */
export function ImpactCategoryLists({
  impact,
  onSymbolClick,
}: {
  impact: ImpactGraph;
  onSymbolClick?: (key: string) => void;
}) {
  return (
    <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
      {impact.categories.map((cat) => (
        <section
          key={cat.name}
          aria-label={`Impact: ${cat.name}`}
          className="rounded-md border p-3"
        >
          <h3 className="mb-1 text-sm font-medium capitalize">
            {cat.name} ({cat.items.length})
          </h3>
          {cat.items.length === 0 ? (
            <p className="text-xs text-muted-foreground">None</p>
          ) : (
            <ul className="space-y-0.5 text-xs">
              {cat.items.map((item) => (
                <li key={item.node.key} className="flex justify-between gap-2">
                  <button
                    type="button"
                    className="truncate font-mono hover:underline"
                    onClick={() => onSymbolClick?.(item.node.key)}
                  >
                    {item.node.qualified_name ?? item.node.name}
                  </button>
                  <span className="shrink-0 text-muted-foreground tabular-nums">
                    d{item.depth} · {item.confidence.toFixed(2)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
      ))}
    </div>
  );
}

/** "Show impact for review": the impact of a changed symbol in a review run. */
export function ImpactGroups({
  reviewId,
  symbolKey,
  onSymbolClick,
}: {
  reviewId: string;
  symbolKey: string;
  onSymbolClick?: (key: string) => void;
}) {
  const query = useQuery({
    queryKey: ['review', reviewId, 'impact', symbolKey],
    queryFn: ({ signal }) => getReviewImpact(reviewId, symbolKey, { signal }),
    throwOnError: false,
    staleTime: Infinity,
  });
  if (query.isPending) return <Loading label="Loading impact" />;
  if (query.isError) return <ErrorState error={query.error} onRetry={() => void query.refetch()} />;
  const impact = query.data;
  return (
    <div className="space-y-4">
      <p className="text-sm">
        Impact of{' '}
        <span className="font-mono">{impact.root.qualified_name ?? impact.root.name}</span>
        {impact.truncated && <span className="ml-2 text-muted-foreground">(truncated)</span>}
      </p>
      <ImpactCategoryLists impact={impact} onSymbolClick={onSymbolClick} />
      {impact.paths.length > 0 && <PathTabs paths={impact.paths} onSymbolClick={onSymbolClick} />}
    </div>
  );
}
