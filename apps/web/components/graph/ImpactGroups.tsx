'use client';

import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { ImpactPathFlow } from '@/components/finding/ImpactPathFlow';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Loading } from '@/components/ui/states';
import { fetchImpact } from '@/lib/api/endpoints';
import type { ImpactCategory, ImpactResponse } from '@/lib/api/pending';
import { isGraphUnavailable } from './SymbolSearch';

const CATEGORY_ORDER: [ImpactCategory, string][] = [
  ['callers', 'Callers'],
  ['callees', 'Callees'],
  ['implementations', 'Implementations'],
  ['interfaces', 'Interfaces'],
  ['overrides', 'Overrides'],
  ['related_types', 'Related types'],
  ['tests', 'Tests'],
  ['entrypoints', 'API entrypoints'],
  ['config', 'Config'],
  ['database', 'Database'],
  ['queues', 'Queues'],
];

/** The ImpactGraph categories as grouped lists, plus the entrypoint path flow. */
export function ImpactGroups({
  impact,
  onSymbolClick,
}: {
  impact: ImpactResponse;
  onSymbolClick?: (key: string) => void;
}) {
  const groups = CATEGORY_ORDER.filter(([id]) => (impact.categories[id]?.length ?? 0) > 0);
  return (
    <div className="space-y-4">
      {impact.path && <ImpactPathFlow path={impact.path} onSymbolClick={onSymbolClick} />}
      {groups.length === 0 && <p className="text-sm text-muted-foreground">No impact found.</p>}
      {groups.map(([id, title]) => (
        <section key={id} aria-label={title}>
          <h4 className="text-xs font-medium text-muted-foreground uppercase">
            {title} ({impact.categories[id]?.length})
          </h4>
          <ul className="divide-y rounded-md border text-sm">
            {impact.categories[id]?.map((item) => (
              <li key={item.key} className="flex items-center justify-between gap-2 px-3 py-1.5">
                <button
                  type="button"
                  className="truncate font-mono text-xs hover:underline"
                  onClick={() => onSymbolClick?.(item.key)}
                >
                  {item.qualified_name ?? item.name}
                </button>
                <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                  distance {item.distance} · confidence {item.min_confidence.toFixed(2)}
                </span>
              </li>
            ))}
          </ul>
        </section>
      ))}
      {impact.truncated && (
        <p className="text-xs text-muted-foreground">Impact truncated by the graph budget.</p>
      )}
    </div>
  );
}

/** "Show impact for review": a review run id plus the selected symbol. */
export function ImpactForReview({
  symbol,
  review,
  onReviewChange,
  onSymbolClick,
}: {
  symbol?: string;
  review?: string;
  onReviewChange: (review: string) => void;
  onSymbolClick?: (key: string) => void;
}) {
  const [draft, setDraft] = useState(review ?? '');
  const query = useQuery({
    queryKey: ['graph', 'impact', review, symbol],
    queryFn: ({ signal }) => fetchImpact(review ?? '', symbol ?? '', { signal }),
    enabled: !!review && !!symbol,
    staleTime: Infinity,
    throwOnError: false,
  });
  return (
    <section aria-label="Impact for review" className="space-y-3">
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (draft.trim()) onReviewChange(draft.trim());
        }}
      >
        <Input
          aria-label="Review id"
          placeholder="Review id"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <Button type="submit" disabled={!draft.trim()}>
          Show impact
        </Button>
      </form>
      {!symbol && (
        <p className="text-sm text-muted-foreground">Select a symbol to see its impact.</p>
      )}
      {query.isFetching && <Loading label="Loading impact" />}
      {query.isError && (
        <p role="alert" className="text-sm text-destructive">
          {isGraphUnavailable(query.error) ? 'Graph service unavailable.' : 'Impact not found.'}
        </p>
      )}
      {query.isSuccess && <ImpactGroups impact={query.data} onSymbolClick={onSymbolClick} />}
    </section>
  );
}
