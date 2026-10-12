'use client';

import { useState } from 'react';
import { ImpactPathFlow } from '@/components/finding/ImpactPathFlow';
import type { GraphPath } from '@/lib/api/pending';
import { pathMinConfidence } from '@/lib/impact-flow';
import { cn } from '@/lib/utils';

/** Up to five alternative paths as tabs, each rendered with the shared `ImpactPathFlow`. */
export function PathTabs({
  paths,
  onSymbolClick,
}: {
  paths: GraphPath[];
  onSymbolClick?: (key: string) => void;
}) {
  const [active, setActive] = useState(0);
  const current = paths[Math.min(active, paths.length - 1)];
  if (!current) return null;
  return (
    <div className="space-y-2">
      {paths.length > 1 && (
        <div role="tablist" aria-label="Alternative paths" className="flex gap-1 border-b">
          {paths.map((p, i) => (
            <button
              key={i}
              type="button"
              role="tab"
              aria-selected={i === active}
              className={cn(
                '-mb-px border-b-2 px-3 py-1.5 text-sm',
                i === active
                  ? 'border-primary font-medium'
                  : 'border-transparent text-muted-foreground',
              )}
              onClick={() => setActive(i)}
            >
              Path {i + 1}{' '}
              <span className="text-xs text-muted-foreground">
                ({p.nodes.length - 1} hops, min{' '}
                {(p.min_confidence ?? pathMinConfidence(p)).toFixed(2)})
              </span>
            </button>
          ))}
        </div>
      )}
      <div role={paths.length > 1 ? 'tabpanel' : undefined}>
        <ImpactPathFlow key={active} path={current} onSymbolClick={onSymbolClick} />
      </div>
    </div>
  );
}
