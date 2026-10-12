import { useEffect } from 'react';
import type { CytoscapeGraphProps } from '../components/graph/CytoscapeGraph';
import { toElements } from '../lib/subgraph';

/**
 * Stand-in for the canvas renderer (jsdom has no canvas): lists the same Cytoscape elements
 * as DOM so tests can assert on them and trigger double-click expansion.
 */
export function CytoscapeGraph({ graph, seeds, onExpand, registerExport }: CytoscapeGraphProps) {
  useEffect(() => {
    registerExport?.(async () => new Blob(['png'], { type: 'image/png' }));
    return () => registerExport?.(null);
  }, [registerExport]);
  const elements = toElements(graph, seeds);
  return (
    <div data-testid="cytoscape">
      <ul aria-label="Graph nodes">
        {elements
          .filter((e) => !e.data.source)
          .map((e) => (
            <li key={e.data.id}>
              <button type="button" onDoubleClick={() => onExpand?.(e.data.id)}>
                {e.data.label}
              </button>
            </li>
          ))}
      </ul>
      <ul aria-label="Graph edges">
        {elements
          .filter((e) => e.data.source)
          .map((e) => (
            <li key={e.data.id} data-low={e.data.low ? 'true' : 'false'}>
              {e.data.id}
            </li>
          ))}
      </ul>
    </div>
  );
}
