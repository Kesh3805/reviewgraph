'use client';

import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  explorerSearch,
  parseExplorerState,
  type ExplorerMode,
  type ExplorerState,
} from '@/lib/explorer-url';
import { cn } from '@/lib/utils';
import { ImpactGroups } from './ImpactGroups';
import { PathFinder } from './PathFinder';
import { SnapshotSelect } from './SnapshotSelect';
import { SubgraphView } from './SubgraphView';
import { SymbolPanel } from './SymbolPanel';
import { SymbolSearch } from './SymbolSearch';

const MODES: { value: ExplorerMode; label: string }[] = [
  { value: 'search', label: 'Symbols' },
  { value: 'path', label: 'Path' },
  { value: 'impact', label: 'Impact for review' },
];

function ReviewInput({ value, onChange }: { value?: string; onChange: (id: string) => void }) {
  const [text, setText] = useState(value ?? '');
  return (
    <form
      className="flex items-center gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (text.trim()) onChange(text.trim());
      }}
    >
      <Input
        aria-label="Review id"
        placeholder="Review run id"
        className="w-80"
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      <Button type="submit" size="sm" variant="outline">
        Use review
      </Button>
    </form>
  );
}

/**
 * CodeGraph Explorer for one repository: symbol search with the symbol panel and subgraph
 * (GX-001/002), the path finder and the review impact view (GX-003). All state lives in the URL
 * (`?mode=&symbol=&snapshot=&review=`), so Finding Detail can deep-link into it.
 */
export function GraphExplorer({ repoId }: { repoId: string }) {
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();
  const state = parseExplorerState(params);
  const update = (patch: Partial<ExplorerState>) =>
    router.replace(`${pathname}${explorerSearch({ ...state, ...patch })}`);
  const openSymbol = (key: string) => update({ mode: 'search', symbol: key });

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div role="tablist" aria-label="Explorer mode" className="flex gap-1">
          {MODES.map((m) => (
            <button
              key={m.value}
              type="button"
              role="tab"
              aria-selected={state.mode === m.value}
              className={cn(
                'rounded-md px-3 py-1.5 text-sm',
                state.mode === m.value
                  ? 'bg-accent font-medium'
                  : 'text-muted-foreground hover:bg-accent/60',
              )}
              onClick={() => update({ mode: m.value })}
            >
              {m.label}
            </button>
          ))}
        </div>
        <SnapshotSelect
          repoId={repoId}
          value={state.snapshot}
          onChange={(snapshot) => update({ snapshot })}
        />
      </div>

      {state.mode === 'search' && (
        <div className="grid gap-6 lg:grid-cols-[minmax(18rem,26rem)_1fr]">
          <SymbolSearch
            repoId={repoId}
            snapshot={state.snapshot}
            selectedKey={state.symbol}
            onSelect={(r) => update({ symbol: r.key })}
          />
          <div className="min-w-0">
            {state.symbol ? (
              <div className="space-y-4">
                <SymbolPanel repoId={repoId} symbolKey={state.symbol} snapshot={state.snapshot} />
                <SubgraphView
                  key={`${state.symbol}@${state.snapshot ?? ''}`}
                  repoId={repoId}
                  seed={state.symbol}
                  snapshot={state.snapshot}
                />
              </div>
            ) : (
              <p className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
                Search for a symbol to inspect it.
              </p>
            )}
          </div>
        </div>
      )}

      {state.mode === 'path' && (
        <PathFinder repoId={repoId} snapshot={state.snapshot} onSymbolClick={openSymbol} />
      )}

      {state.mode === 'impact' && (
        <div className="space-y-4">
          <ReviewInput value={state.review} onChange={(review) => update({ review })} />
          <div className="grid gap-6 lg:grid-cols-[minmax(18rem,26rem)_1fr]">
            <SymbolSearch
              repoId={repoId}
              snapshot={state.snapshot}
              selectedKey={state.symbol}
              label="Changed symbol"
              onSelect={(r) => update({ symbol: r.key })}
            />
            <div className="min-w-0">
              {state.review && state.symbol ? (
                <ImpactGroups
                  reviewId={state.review}
                  symbolKey={state.symbol}
                  onSymbolClick={openSymbol}
                />
              ) : (
                <p className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
                  Pick a review and a changed symbol to see its impact.
                </p>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
