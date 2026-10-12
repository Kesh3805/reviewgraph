'use client';

import { Background, Controls, ReactFlow, type NodeMouseHandler } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useMemo } from 'react';
import type { GraphPath } from '@/lib/api/pending';
import { buildFlow, edgeLabel, isLowConfidence, pathMinConfidence } from '@/lib/impact-flow';

/**
 * A curated impact path rendered with React Flow, left to right from the entrypoint to the
 * changed symbol. Edge labels show kind and confidence; edges below 0.6 are dashed. Reused by
 * Finding Detail (WEB-007) and the explorer path view (GX-003).
 */
export function ImpactPathFlow({
  path,
  onSymbolClick,
  height = 260,
}: {
  path: GraphPath;
  /** Called with the symbol key when a node is clicked. */
  onSymbolClick?: (symbolKey: string) => void;
  height?: number;
}) {
  const flow = useMemo(() => buildFlow(path), [path]);
  const onNodeClick: NodeMouseHandler = (_event, node) => onSymbolClick?.(node.id);
  const minConfidence = path.min_confidence ?? pathMinConfidence(path);

  return (
    <div className="space-y-2">
      <div className="rounded-md border" style={{ height }} data-testid="impact-flow">
        <ReactFlow
          nodes={flow.nodes}
          edges={flow.edges}
          onNodeClick={onNodeClick}
          nodesDraggable={false}
          nodesConnectable={false}
          fitView
          proOptions={{ hideAttribution: true }}
        >
          <Background />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>
      {/* The same chain as text: readable without the canvas and for assistive technology. */}
      <ol aria-label="Impact path" className="flex flex-wrap items-center gap-1 text-xs">
        {path.nodes.map((node, i) => {
          const edge = path.edges[i];
          return (
            <li key={node.key} className="flex items-center gap-1">
              <button
                type="button"
                className="rounded bg-muted px-1.5 py-0.5 font-mono hover:underline"
                onClick={() => onSymbolClick?.(node.key)}
              >
                {node.qualified_name ?? node.name}
              </button>
              {edge && i < path.nodes.length - 1 && (
                <span
                  className="text-muted-foreground"
                  data-dashed={isLowConfidence(edge.confidence) ? 'true' : undefined}
                >
                  —{edgeLabel(edge.kind, edge.confidence)}→
                </span>
              )}
            </li>
          );
        })}
      </ol>
      <p className="text-xs text-muted-foreground">
        Path minimum confidence {minConfidence.toFixed(2)}
      </p>
    </div>
  );
}
