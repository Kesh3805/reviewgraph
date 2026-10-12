'use client';

import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { ImpactPathFlow } from '@/components/finding/ImpactPathFlow';
import { Button } from '@/components/ui/button';
import { Loading } from '@/components/ui/states';
import { fetchPaths } from '@/lib/api/endpoints';
import type { SymbolSearchResult } from '@/lib/api/pending';
import { isGraphUnavailable, SymbolSearch } from './SymbolSearch';

const RELATION_KINDS = ['CALLS', 'IMPORTS', 'IMPLEMENTS', 'EXTENDS', 'HANDLED_BY', 'TESTS'];

interface Submitted {
  from: string;
  to: string;
  kinds: string[];
  depth: number;
}

function Picked({ label, symbol }: { label: string; symbol: SymbolSearchResult | null }) {
  return (
    <p className="text-xs" data-testid={`picked-${label.toLowerCase()}`}>
      <span className="font-medium">{label}:</span>{' '}
      <span className="font-mono">{symbol ? symbol.qualified_name : 'not selected'}</span>
    </p>
  );
}

/**
 * "How does A reach B?": pick FROM and TO, relation kinds and a depth (at most 6). Up to five
 * alternative paths come back and are shown as tabs, each rendered with `ImpactPathFlow`.
 */
export function PathFinder({
  repoId,
  snapshot,
  onSymbolClick,
}: {
  repoId: string;
  snapshot?: string;
  onSymbolClick?: (key: string) => void;
}) {
  const [from, setFrom] = useState<SymbolSearchResult | null>(null);
  const [to, setTo] = useState<SymbolSearchResult | null>(null);
  const [kinds, setKinds] = useState<string[]>([]);
  const [depth, setDepth] = useState(4);
  const [submitted, setSubmitted] = useState<Submitted | null>(null);
  const [tab, setTab] = useState(0);

  const query = useQuery({
    queryKey: ['graph', repoId, snapshot ?? 'default', 'paths', submitted],
    queryFn: ({ signal }) =>
      fetchPaths(
        repoId,
        {
          from: submitted?.from ?? '',
          to: submitted?.to ?? '',
          kinds: submitted?.kinds,
          max_depth: submitted?.depth ?? 4,
          snapshot,
        },
        { signal },
      ),
    enabled: submitted !== null,
    staleTime: Infinity,
    throwOnError: false,
  });

  const toggle = (k: string) =>
    setKinds((prev) => (prev.includes(k) ? prev.filter((x) => x !== k) : [...prev, k]));

  const paths = query.data?.paths ?? [];
  const active = paths[Math.min(tab, Math.max(paths.length - 1, 0))];

  return (
    <section aria-label="Path finder" className="space-y-4">
      <div className="grid gap-4 md:grid-cols-2">
        <div className="space-y-2">
          <Picked label="From" symbol={from} />
          <SymbolSearch
            repoId={repoId}
            snapshot={snapshot}
            label="From symbol"
            onSelect={setFrom}
          />
        </div>
        <div className="space-y-2">
          <Picked label="To" symbol={to} />
          <SymbolSearch repoId={repoId} snapshot={snapshot} label="To symbol" onSelect={setTo} />
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex flex-wrap gap-1" role="group" aria-label="Relation kinds">
          {RELATION_KINDS.map((k) => (
            <Button
              key={k}
              type="button"
              size="sm"
              variant={kinds.includes(k) ? 'default' : 'outline'}
              aria-pressed={kinds.includes(k)}
              onClick={() => toggle(k)}
            >
              {k}
            </Button>
          ))}
        </div>
        <label className="flex items-center gap-2 text-sm">
          Max depth
          <select
            className="rounded-md border bg-background px-2 py-1"
            value={depth}
            onChange={(e) => setDepth(Number(e.target.value))}
          >
            {[1, 2, 3, 4, 5, 6].map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
        </label>
        <Button
          type="button"
          disabled={!from || !to}
          onClick={() => {
            if (!from || !to) return;
            setTab(0);
            setSubmitted({ from: from.key, to: to.key, kinds, depth });
          }}
        >
          Find path
        </Button>
      </div>

      {query.isFetching && <Loading label="Finding paths" />}
      {query.isError && (
        <p role="alert" className="text-sm text-destructive">
          {isGraphUnavailable(query.error)
            ? 'Graph service unavailable.'
            : 'Could not find a path.'}
        </p>
      )}
      {query.isSuccess && !query.isFetching && paths.length === 0 && submitted && (
        <p role="status" className="rounded-md border border-dashed p-4 text-sm">
          No path within depth {submitted.depth}.
        </p>
      )}
      {query.isSuccess && !query.isFetching && active && (
        <div className="space-y-2">
          {paths.length > 1 && (
            <div role="tablist" aria-label="Alternative paths" className="flex gap-1">
              {paths.map((p, i) => (
                <Button
                  key={i}
                  type="button"
                  role="tab"
                  size="sm"
                  aria-selected={i === tab}
                  variant={i === tab ? 'default' : 'outline'}
                  onClick={() => setTab(i)}
                >
                  Path {i + 1} ({p.edges.length} hops)
                </Button>
              ))}
            </div>
          )}
          <ImpactPathFlow path={active} onSymbolClick={onSymbolClick} />
        </div>
      )}
    </section>
  );
}
