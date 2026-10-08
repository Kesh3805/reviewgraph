'use client';

import { useMutation, useQuery } from '@tanstack/react-query';
import dynamic from 'next/dynamic';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Loading } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { fetchSubgraph } from '@/lib/api/endpoints';
import type { SubgraphRelation, SubgraphRequest } from '@/lib/api/pending';
import { EMPTY_GRAPH, MAX_SUBGRAPH_NODES, mergeSubgraph, type ClientGraph } from '@/lib/subgraph';
import type { PngExporter } from './CytoscapeGraph';
import { GraphToolbar } from './GraphToolbar';
import { isGraphUnavailable } from './SymbolSearch';

const CytoscapeGraph = dynamic(() => import('./CytoscapeGraph').then((m) => m.CytoscapeGraph), {
  ssr: false,
  loading: () => (
    <Loading
      label="Loading graph renderer"
      className="h-[520px] animate-pulse rounded-md bg-muted"
    />
  ),
});

type Banner = { kind: 'truncated' | 'refused' | 'budget' | 'error'; message: string } | null;

function failureBanner(error: unknown): Banner {
  if (error instanceof ApiError && error.status === 400) {
    return {
      kind: 'budget',
      message: 'The request exceeds the graph budget. Narrow the relations or depth.',
    };
  }
  return {
    kind: 'error',
    message: isGraphUnavailable(error)
      ? 'Graph service unavailable.'
      : 'Could not load the subgraph.',
  };
}

function download(blob: Blob, name: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  URL.revokeObjectURL(url);
}

/**
 * Server-side subgraph around a seed symbol. Double-click expands a node (one expansion in
 * flight at a time); merges dedupe by key and never exceed 500 nodes.
 */
export function SubgraphView({
  repoId,
  seed,
  snapshot,
  onSelect,
}: {
  repoId: string;
  seed: string;
  snapshot?: string;
  onSelect?: (key: string) => void;
}) {
  const [relations, setRelations] = useState<SubgraphRelation[]>(['callers', 'callees']);
  const [depth, setDepth] = useState(1);
  const [graph, setGraph] = useState<ClientGraph>(EMPTY_GRAPH);
  const [banner, setBanner] = useState<Banner>(null);
  const exporter = useRef<PngExporter | null>(null);
  const [canExport, setCanExport] = useState(false);
  const seeds = useMemo(() => [seed], [seed]);

  const base: SubgraphRequest = useMemo(
    () => ({
      seeds,
      depth,
      kinds: relations,
      max_nodes: MAX_SUBGRAPH_NODES,
      ...(snapshot ? { snapshot } : {}),
    }),
    [seeds, depth, relations, snapshot],
  );
  // Any toggle, depth or seed change refetches from scratch.
  const initial = useQuery({
    queryKey: ['graph', repoId, 'subgraph', base],
    queryFn: ({ signal }) => fetchSubgraph(repoId, base, { signal }),
    throwOnError: false,
    staleTime: Infinity,
  });

  useEffect(() => {
    if (initial.data) {
      setGraph({ nodes: initial.data.nodes, edges: initial.data.edges });
      setBanner(
        initial.data.truncated
          ? { kind: 'truncated', message: 'The subgraph was truncated at the server budget.' }
          : null,
      );
    } else if (initial.error) {
      setGraph(EMPTY_GRAPH);
      setBanner(failureBanner(initial.error));
    }
  }, [initial.data, initial.error]);

  const expand = useMutation({
    mutationFn: (key: string) =>
      fetchSubgraph(repoId, {
        ...base,
        seeds: [key],
        depth: 1,
        max_nodes: MAX_SUBGRAPH_NODES,
      }),
    onSuccess: (result) => {
      const merged = mergeSubgraph(graph, result);
      if (merged.status === 'refused') {
        setBanner({
          kind: 'refused',
          message: `Expanding would show ${merged.wouldHave} nodes (limit ${MAX_SUBGRAPH_NODES}). Narrow the relations or reset first.`,
        });
        return;
      }
      setGraph(merged.graph);
      setBanner(
        result.truncated
          ? { kind: 'truncated', message: 'The expansion was truncated at the server budget.' }
          : null,
      );
    },
    onError: (error) => setBanner(failureBanner(error)),
  });

  const onExpand = useCallback(
    (key: string) => {
      // At most one expansion request in flight.
      if (expand.isPending) return;
      if (graph.nodes.length >= MAX_SUBGRAPH_NODES) {
        setBanner({
          kind: 'refused',
          message: `The graph already has ${MAX_SUBGRAPH_NODES} nodes. Narrow the relations or reset first.`,
        });
        return;
      }
      expand.mutate(key);
    },
    [expand, graph.nodes.length],
  );

  const registerExport = useCallback((fn: PngExporter | null) => {
    exporter.current = fn;
    setCanExport(!!fn);
  }, []);

  async function exportPng() {
    const blob = await exporter.current?.();
    if (blob) download(blob, `graph-${seed.slice(0, 24)}.png`);
  }

  return (
    <section aria-label="Subgraph" className="space-y-2">
      <GraphToolbar
        relations={relations}
        onRelationsChange={setRelations}
        depth={depth}
        onDepthChange={setDepth}
        onReset={() =>
          void initial
            .refetch()
            .then((r) => r.data && setGraph({ nodes: r.data.nodes, edges: r.data.edges }))
        }
        onExport={canExport ? () => void exportPng() : undefined}
        nodeCount={graph.nodes.length}
        maxNodes={MAX_SUBGRAPH_NODES}
      />
      {banner && (
        <p
          role={banner.kind === 'truncated' ? 'status' : 'alert'}
          data-testid={`banner-${banner.kind}`}
          className="rounded-md border border-amber-500/40 p-2 text-sm"
        >
          {banner.message}
        </p>
      )}
      {expand.isPending && (
        <p role="status" className="text-xs text-muted-foreground">
          Expanding…
        </p>
      )}
      {initial.isPending ? (
        <Loading label="Loading subgraph" className="h-[520px] animate-pulse rounded-md bg-muted" />
      ) : (
        <CytoscapeGraph
          graph={graph}
          seeds={seeds}
          onExpand={onExpand}
          onSelect={onSelect}
          registerExport={registerExport}
        />
      )}
      <p className="text-xs text-muted-foreground">
        Double-click a node to expand it. Edge width follows confidence; dashed edges are below 0.6.
      </p>
    </section>
  );
}
