import type { StylesheetJson } from 'cytoscape';

/** Node colour and shape per symbol kind. */
export const NODE_KIND_STYLE: Record<string, { color: string; shape: string }> = {
  class: { color: '#6366f1', shape: 'round-rectangle' },
  interface: { color: '#8b5cf6', shape: 'round-rectangle' },
  method: { color: '#0ea5e9', shape: 'ellipse' },
  function: { color: '#06b6d4', shape: 'ellipse' },
  endpoint: { color: '#f97316', shape: 'diamond' },
  queue: { color: '#eab308', shape: 'hexagon' },
  table: { color: '#22c55e', shape: 'barrel' },
  test: { color: '#a3a3a3', shape: 'tag' },
  module: { color: '#64748b', shape: 'round-rectangle' },
};

/** Cytoscape stylesheet: kind styles, width by confidence, dashed below 0.6. */
export function graphStylesheet(): StylesheetJson {
  return [
    {
      selector: 'node',
      style: {
        label: 'data(label)',
        'font-size': 9,
        'text-valign': 'bottom',
        'text-margin-y': 4,
        color: '#64748b',
        width: 18,
        height: 18,
        'background-color': '#94a3b8',
      },
    },
    ...Object.entries(NODE_KIND_STYLE).map(([kind, s]) => ({
      selector: `node[kind = "${kind}"]`,
      style: { 'background-color': s.color, shape: s.shape },
    })),
    { selector: 'node[?seed]', style: { 'border-width': 3, 'border-color': '#0f172a' } },
    {
      selector: 'edge',
      style: {
        width: 'mapData(confidence, 0, 1, 1, 5)',
        'line-color': '#94a3b8',
        'target-arrow-color': '#94a3b8',
        'target-arrow-shape': 'triangle',
        'curve-style': 'bezier',
        'arrow-scale': 0.7,
      },
    },
    { selector: 'edge[?low]', style: { 'line-style': 'dashed' } },
  ] as StylesheetJson;
}
