'use client';

import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Select } from '@/components/ui/input';
import { findPaths } from '@/lib/api/endpoints';
import type { PathQuery, SymbolSearchResult } from '@/lib/api/pending';
import { PathTabs } from './PathTabs';
import { SymbolSearch, isGraphUnavailable } from './SymbolSearch';

export const PATH_EDGE_KINDS = [
  'CALLS',
  'IMPLEMENTS',
  'DEPENDS_ON',
  'IMPORTS',
  'PUBLISHES',
  'CONSUMES',
];
export const MAX_PATH_DEPTH = 6;
export const MAX_ALTERNATIVE_PATHS = 5;

function Picked({
  label,
  value,
  onClear,
}: {
  label: string;
  value: SymbolSearchResult;
  onClear: () => void;
}) {
  return (
    <p className="flex items-center justify-between gap-2 rounded-md border px-3 py-2 text-sm">
      <span>
        <span className="text-muted-foreground">{label}:</span>{' '}
        <span className="font-mono text-xs">{value.qualified_name}</span>
      </span>
      <Button size="sm" variant="ghost" onClick={onClear} aria-label={`Change ${label}`}>
        Change
      </Button>
    </p>
  );
}

/** "How does A reach B?": pick FROM and TO, edge kinds and max depth (≤ 6). */
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
  const [kinds, setKinds] = useState<string[]>(['CALLS']);
  const [maxDepth, setMaxDepth] = useState(MAX_PATH_DEPTH);
  const [submitted, setSubmitted] = useState<PathQuery | null>(null);

  const query = useQuery({
    queryKey: ['graph', repoId, 'path', submitted],
    queryFn: ({ signal }) => findPaths(repoId, submitted as PathQuery, { signal }),
    enabled: !!submitted,
    throwOnError: false,
    staleTime: Infinity,
  });

  function find() {
    if (!from || !to) return;
    setSubmitted({
      from: from.key,
      to: to.key,
      max_depth: maxDepth,
      kinds: kinds.length > 0 ? kinds.join(',') : undefined,
      all: true,
      max_paths: MAX_ALTERNATIVE_PATHS,
      ...(snapshot ? { snapshot } : {}),
    });
  }

  const toggle = (k: string) =>
    setKinds((prev) => (prev.includes(k) ? prev.filter((x) => x !== k) : [...prev, k]));

  return (
    <section aria-label="Path finder" className="space-y-4">
      <div className="grid gap-4 md:grid-cols-2">
        {from ? (
          <Picked label="From" value={from} onClear={() => setFrom(null)} />
        ) : (
          <SymbolSearch
            repoId={repoId}
            snapshot={snapshot}
            label="From symbol"
            onSelect={setFrom}
          />
        )}
        {to ? (
          <Picked label="To" value={to} onClear={() => setTo(null)} />
        ) : (
          <SymbolSearch repoId={repoId} snapshot={snapshot} label="To symbol" onSelect={setTo} />
        )}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <div role="group" aria-label="Edge kinds" className="flex flex-wrap gap-1">
          {PATH_EDGE_KINDS.map((k) => (
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
        <Select
          aria-label="Max depth"
          className="w-32"
          value={maxDepth}
          onChange={(e) => setMaxDepth(Number(e.target.value))}
        >
          {Array.from({ length: MAX_PATH_DEPTH }, (_, i) => i + 1).map((d) => (
            <option key={d} value={d}>
              max depth {d}
            </option>
          ))}
        </Select>
        <Button type="button" disabled={!from || !to || query.isFetching} onClick={find}>
          Find path
        </Button>
      </div>

      {query.isFetching && (
        <p role="status" className="text-sm text-muted-foreground">
          Searching paths…
        </p>
      )}
      {query.isError && (
        <p role="alert" className="text-sm text-destructive">
          {isGraphUnavailable(query.error)
            ? 'Graph service unavailable.'
            : 'Could not compute the path.'}
        </p>
      )}
      {query.data && submitted && query.data.paths.length === 0 && (
        <p role="status" className="rounded-md border border-dashed p-4 text-sm">
          No path within depth {submitted.max_depth}.
        </p>
      )}
      {query.data && query.data.paths.length > 0 && (
        <PathTabs paths={query.data.paths} onSymbolClick={onSymbolClick} />
      )}
    </section>
  );
}
