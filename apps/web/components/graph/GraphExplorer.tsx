'use client';

import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { explorerSearch, parseExplorerState, type ExplorerState } from '@/lib/explorer-url';
import { SnapshotSelect } from './SnapshotSelect';
import { SymbolPanel } from './SymbolPanel';
import { SymbolSearch } from './SymbolSearch';

/**
 * CodeGraph Explorer for one repository. All state lives in the URL
 * (`?symbol=<key>&snapshot=<id>`), so Finding Detail can deep-link into it.
 */
export function GraphExplorer({ repoId }: { repoId: string }) {
  const router = useRouter();
  const pathname = usePathname();
  const params = useSearchParams();
  const state = parseExplorerState(params);
  const update = (patch: Partial<ExplorerState>) =>
    router.replace(`${pathname}${explorerSearch({ ...state, ...patch })}`);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <SnapshotSelect
          repoId={repoId}
          value={state.snapshot}
          onChange={(snapshot) => update({ snapshot })}
        />
      </div>
      <div className="grid gap-6 lg:grid-cols-[minmax(18rem,26rem)_1fr]">
        <SymbolSearch
          repoId={repoId}
          snapshot={state.snapshot}
          selectedKey={state.symbol}
          onSelect={(r) => update({ symbol: r.key })}
        />
        <div className="min-w-0">
          {state.symbol ? (
            <SymbolPanel repoId={repoId} symbolKey={state.symbol} snapshot={state.snapshot} />
          ) : (
            <p className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
              Search for a symbol to inspect it.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
