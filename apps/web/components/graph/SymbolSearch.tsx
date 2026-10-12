'use client';

import { useEffect, useMemo, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ApiError } from '@/lib/api-client';
import { searchSymbols } from '@/lib/api/endpoints';
import type { SymbolKindFilter, SymbolSearchResponse, SymbolSearchResult } from '@/lib/api/pending';
import { DebouncedSearch } from '@/lib/debounced-search';
import { cn } from '@/lib/utils';

export const KIND_FILTERS: SymbolKindFilter[] = [
  'class',
  'method',
  'function',
  'endpoint',
  'queue',
  'table',
  'test',
];

interface Query {
  q: string;
  kinds: SymbolKindFilter[];
  snapshot?: string;
}

type State =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'done'; result: SymbolSearchResponse }
  | { status: 'unavailable' }
  | { status: 'error'; message: string };

export function isGraphUnavailable(error: unknown): boolean {
  return error instanceof ApiError ? error.status === 503 || error.status === 502 : true;
}

/**
 * Debounced (200 ms) symbol search with kind filters. Each new query aborts the previous
 * request. Results show the qualified name, kind, path:line and in/out degree.
 */
export function SymbolSearch({
  repoId,
  snapshot,
  selectedKey,
  onSelect,
  label = 'Search symbols',
}: {
  repoId: string;
  snapshot?: string;
  selectedKey?: string | null;
  onSelect: (result: SymbolSearchResult) => void;
  label?: string;
}) {
  const [text, setText] = useState('');
  const [kinds, setKinds] = useState<SymbolKindFilter[]>([]);
  const [state, setState] = useState<State>({ status: 'idle' });
  const [attempt, setAttempt] = useState(0);

  const search = useMemo(
    () =>
      new DebouncedSearch<Query, SymbolSearchResponse>(
        (query, signal) => searchSymbols(repoId, query, { signal }),
        (result) => setState({ status: 'done', result }),
        (error) =>
          setState(
            isGraphUnavailable(error)
              ? { status: 'unavailable' }
              : {
                  status: 'error',
                  message: error instanceof Error ? error.message : 'Search failed.',
                },
          ),
      ),
    [repoId],
  );

  useEffect(() => () => search.cancel(), [search]);

  useEffect(() => {
    const q = text.trim();
    if (!q) {
      search.cancel();
      setState({ status: 'idle' });
      return;
    }
    const query: Query = { q, kinds, snapshot };
    setState({ status: 'loading' });
    search.search(query);
  }, [text, kinds, snapshot, search, attempt]);

  const toggleKind = (kind: SymbolKindFilter) =>
    setKinds((prev) => (prev.includes(kind) ? prev.filter((k) => k !== kind) : [...prev, kind]));

  return (
    <div className="space-y-2">
      <Input
        type="search"
        aria-label={label}
        placeholder="AuthService.authorize"
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="flex flex-wrap gap-1" role="group" aria-label="Kinds">
        {KIND_FILTERS.map((kind) => (
          <Button
            key={kind}
            type="button"
            size="sm"
            variant={kinds.includes(kind) ? 'default' : 'outline'}
            aria-pressed={kinds.includes(kind)}
            onClick={() => toggleKind(kind)}
          >
            {kind}
          </Button>
        ))}
      </div>
      {state.status === 'loading' && (
        <p role="status" className="text-sm text-muted-foreground">
          Searching…
        </p>
      )}
      {state.status === 'unavailable' && (
        <div
          role="alert"
          className="flex items-center justify-between rounded-md border p-2 text-sm"
        >
          <span className="text-destructive">Graph service unavailable.</span>
          <Button size="sm" variant="outline" onClick={() => setAttempt((n) => n + 1)}>
            Retry
          </Button>
        </div>
      )}
      {state.status === 'error' && (
        <p role="alert" className="text-sm text-destructive">
          {state.message}
        </p>
      )}
      {state.status === 'done' && state.result.items.length === 0 && (
        <p className="text-sm text-muted-foreground">No symbols match.</p>
      )}
      {state.status === 'done' && state.result.items.length > 0 && (
        <ul
          aria-label="Symbol results"
          className="max-h-96 divide-y overflow-y-auto rounded-md border"
        >
          {state.result.items.map((r) => (
            <li key={r.key}>
              <button
                type="button"
                className={cn(
                  'w-full px-3 py-2 text-left text-sm hover:bg-accent',
                  selectedKey === r.key && 'bg-accent',
                )}
                onClick={() => onSelect(r)}
              >
                <span className="flex items-center justify-between gap-2">
                  <span className="truncate font-mono text-xs font-medium">{r.qualified_name}</span>
                  <span className="shrink-0 text-xs text-muted-foreground">{r.kind}</span>
                </span>
                <span className="flex justify-between gap-2 text-xs text-muted-foreground">
                  <span className="truncate">
                    {r.path}:{r.line}
                  </span>
                  <span className="shrink-0 tabular-nums">
                    in {r.in_degree} · out {r.out_degree}
                  </span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {state.status === 'done' && state.result.truncated && (
        <p className="text-xs text-muted-foreground">
          Showing the first 50 matches; refine the query.
        </p>
      )}
    </div>
  );
}
