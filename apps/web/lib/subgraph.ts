import type { GraphEdge, GraphNode, Subgraph } from './api/pending';
import { isLowConfidence } from './impact-flow';

/** The client never holds more than this many nodes (GX-002, API-011 budget). */
export const MAX_SUBGRAPH_NODES = 500;

export interface ClientGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export const EMPTY_GRAPH: ClientGraph = { nodes: [], edges: [] };

export function edgeId(e: GraphEdge): string {
  return `${e.source}->${e.target}:${e.kind}`;
}

export type MergeResult =
  | { status: 'merged'; graph: ClientGraph; added: number }
  | { status: 'refused'; graph: ClientGraph; wouldHave: number };

/**
 * Merges an expansion into the current graph, deduplicating nodes by key and edges by
 * source/target/kind. A merge that would exceed `max` nodes is refused as a whole, so the user
 * is asked to narrow instead of seeing a silently partial graph.
 */
export function mergeSubgraph(
  current: ClientGraph,
  incoming: Pick<Subgraph, 'nodes' | 'edges'>,
  max = MAX_SUBGRAPH_NODES,
): MergeResult {
  const nodes = new Map(current.nodes.map((n) => [n.key, n]));
  let added = 0;
  for (const n of incoming.nodes) {
    if (!nodes.has(n.key)) {
      nodes.set(n.key, n);
      added += 1;
    }
  }
  if (nodes.size > max) return { status: 'refused', graph: current, wouldHave: nodes.size };

  const edges = new Map(current.edges.map((e) => [edgeId(e), e]));
  for (const e of incoming.edges) {
    if (nodes.has(e.source) && nodes.has(e.target) && !edges.has(edgeId(e)))
      edges.set(edgeId(e), e);
  }
  return {
    status: 'merged',
    graph: { nodes: [...nodes.values()], edges: [...edges.values()] },
    added,
  };
}

export interface ElementData {
  id: string;
  label?: string;
  kind: string;
  source?: string;
  target?: string;
  confidence?: number;
  /** Edges below 0.6 confidence render dashed. */
  low?: boolean;
  seed?: boolean;
}

/** Cytoscape element definitions for a graph. */
export function toElements(graph: ClientGraph, seeds: string[] = []): { data: ElementData }[] {
  const seedSet = new Set(seeds);
  return [
    ...graph.nodes.map((n) => ({
      data: {
        id: n.key,
        label: n.qualified_name ?? n.name,
        kind: n.kind,
        seed: seedSet.has(n.key),
      },
    })),
    ...graph.edges.map((e) => ({
      data: {
        id: edgeId(e),
        source: e.source,
        target: e.target,
        kind: e.kind,
        confidence: e.confidence,
        low: isLowConfidence(e.confidence),
      },
    })),
  ];
}
