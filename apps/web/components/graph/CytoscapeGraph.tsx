'use client';

import cytoscape, { type Core } from 'cytoscape';
import fcose from 'cytoscape-fcose';
import { useEffect, useRef } from 'react';
import type { ClientGraph } from '@/lib/subgraph';
import { toElements } from '@/lib/subgraph';
import { graphStylesheet } from './styles';

let registered = false;
function ensureFcose() {
  if (!registered) {
    cytoscape.use(fcose);
    registered = true;
  }
}

/** Renders the current view as a PNG blob. */
export type PngExporter = () => Promise<Blob | null>;

export interface CytoscapeGraphProps {
  graph: ClientGraph;
  seeds: string[];
  /** Double-click on a node. */
  onExpand?: (key: string) => void;
  /** Single click on a node. */
  onSelect?: (key: string) => void;
  height?: number;
  /** Receives the PNG exporter once the graph is mounted (null on unmount). */
  registerExport?: (exporter: PngExporter | null) => void;
}

/**
 * Cytoscape renderer for a server-side subgraph (fcose layout). Loaded with
 * `dynamic(..., { ssr: false })` because cytoscape needs the DOM.
 */
export function CytoscapeGraph({
  graph,
  seeds,
  onExpand,
  onSelect,
  height = 520,
  registerExport,
}: CytoscapeGraphProps) {
  const container = useRef<HTMLDivElement>(null);
  const cy = useRef<Core | null>(null);
  const handlers = useRef({ onExpand, onSelect, registerExport });
  handlers.current = { onExpand, onSelect, registerExport };

  useEffect(() => {
    ensureFcose();
    const instance = cytoscape({
      container: container.current,
      style: graphStylesheet(),
      wheelSensitivity: 0.3,
      minZoom: 0.1,
      maxZoom: 3,
    });
    instance.on('dbltap', 'node', (e) => handlers.current.onExpand?.(e.target.id()));
    instance.on('tap', 'node', (e) => handlers.current.onSelect?.(e.target.id()));
    cy.current = instance;
    handlers.current.registerExport?.(() =>
      instance.png({ output: 'blob-promise', full: true, bg: '#ffffff', scale: 2 }),
    );
    return () => {
      handlers.current.registerExport?.(null);
      instance.destroy();
      cy.current = null;
    };
  }, []);

  useEffect(() => {
    const instance = cy.current;
    if (!instance) return;
    instance.batch(() => {
      instance.elements().remove();
      instance.add(toElements(graph, seeds));
    });
    instance
      .layout({
        name: 'fcose',
        animate: false,
        randomize: true,
        nodeRepulsion: () => 6000,
      } as cytoscape.LayoutOptions)
      .run();
  }, [graph, seeds]);

  return (
    <div
      ref={container}
      role="img"
      aria-label={`Graph with ${graph.nodes.length} nodes and ${graph.edges.length} edges`}
      className="w-full rounded-md border bg-background"
      style={{ height }}
    />
  );
}
