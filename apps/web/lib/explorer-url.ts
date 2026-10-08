/** Explorer view modes (GX-001..GX-003). */
export type ExplorerMode = 'search' | 'path' | 'impact';

/** Deep-linkable explorer state: `?mode=&symbol=&snapshot=&review=`. */
export interface ExplorerState {
  mode: ExplorerMode;
  symbol?: string;
  snapshot?: string;
  /** Review run for the "Show impact for review" mode. */
  review?: string;
}

type Params = { get(name: string): string | null };

export function parseExplorerState(params: Params): ExplorerState {
  const mode = params.get('mode');
  return {
    mode: mode === 'path' || mode === 'impact' ? mode : 'search',
    symbol: params.get('symbol') || undefined,
    snapshot: params.get('snapshot') || undefined,
    review: params.get('review') || undefined,
  };
}

export function explorerSearch(state: ExplorerState): string {
  const p = new URLSearchParams();
  if (state.mode !== 'search') p.set('mode', state.mode);
  if (state.symbol) p.set('symbol', state.symbol);
  if (state.snapshot) p.set('snapshot', state.snapshot);
  if (state.review) p.set('review', state.review);
  const s = p.toString();
  return s ? `?${s}` : '';
}
