import { Graph, layout } from '@dagrejs/dagre';
import { Position, type Edge, type Node } from '@xyflow/react';
import type { GraphPath } from './api/pending';

/** Curated paths only: anything longer is cut (the API already caps at 30). */
export const MAX_FLOW_NODES = 30;
/** Edges below this confidence are dashed (WEB-007, GX-002). */
export const LOW_CONFIDENCE = 0.6;

const NODE_WIDTH = 220;
const NODE_HEIGHT = 56;

export interface FlowNodeData extends Record<string, unknown> {
  label: string;
  kind: string;
  symbolKey: string;
  location: string | null;
  role: 'entrypoint' | 'changed' | 'hop';
}

export function edgeLabel(kind: string, confidence: number): string {
  return `${kind} ${confidence.toFixed(2)}`;
}

export function isLowConfidence(confidence: number): boolean {
  return confidence < LOW_CONFIDENCE;
}

/** Lowest edge confidence along a path (1 for a single node). */
export function pathMinConfidence(path: GraphPath): number {
  return path.edges.reduce((min, e) => Math.min(min, e.confidence), 1);
}

/**
 * React Flow nodes and edges for a path laid out left to right with dagre: the entrypoint is
 * first, the changed symbol last.
 */
export function buildFlow(path: GraphPath): { nodes: Node<FlowNodeData>[]; edges: Edge[] } {
  const pathNodes = path.nodes.slice(0, MAX_FLOW_NODES);
  const keys = new Set(pathNodes.map((n) => n.key));
  const pathEdges = path.edges.filter((e) => keys.has(e.source) && keys.has(e.target));

  const g = new Graph();
  g.setGraph({ rankdir: 'LR', nodesep: 30, ranksep: 70 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const n of pathNodes) g.setNode(n.key, { width: NODE_WIDTH, height: NODE_HEIGHT });
  for (const e of pathEdges) g.setEdge(e.source, e.target);
  layout(g);

  const last = pathNodes.length - 1;
  const nodes: Node<FlowNodeData>[] = pathNodes.map((n, i) => {
    const pos = g.node(n.key) as { x: number; y: number } | undefined;
    return {
      id: n.key,
      position: { x: (pos?.x ?? i * 260) - NODE_WIDTH / 2, y: (pos?.y ?? 0) - NODE_HEIGHT / 2 },
      data: {
        label: n.qualified_name ?? n.name,
        kind: n.kind,
        symbolKey: n.key,
        location: n.path ? `${n.path}${n.line ? `:${n.line}` : ''}` : null,
        role: i === 0 ? 'entrypoint' : i === last ? 'changed' : 'hop',
      },
      sourcePosition: Position.Right,
      targetPosition: Position.Left,
      style: { width: NODE_WIDTH },
    };
  });

  const edges: Edge[] = pathEdges.map((e, i) => {
    const low = isLowConfidence(e.confidence);
    return {
      id: `${e.source}->${e.target}#${i}`,
      source: e.source,
      target: e.target,
      label: edgeLabel(e.kind, e.confidence),
      data: { confidence: e.confidence, kind: e.kind, dashed: low },
      style: low ? { strokeDasharray: '6 4' } : undefined,
      animated: false,
    };
  });

  return { nodes, edges };
}
