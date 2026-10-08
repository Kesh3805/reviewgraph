'use client';

import { Download } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Select } from '@/components/ui/input';
import type { SubgraphRelation } from '@/lib/api/pending';

export const RELATIONS: { value: SubgraphRelation; label: string }[] = [
  { value: 'callers', label: 'Callers' },
  { value: 'callees', label: 'Callees' },
  { value: 'implementations', label: 'Implementations' },
  { value: 'tests', label: 'Tests' },
  { value: 'dependencies', label: 'Dependencies' },
];

/** Relation toggles, depth (1..3), reset and PNG export. */
export function GraphToolbar({
  relations,
  onRelationsChange,
  depth,
  onDepthChange,
  onReset,
  onExport,
  nodeCount,
  maxNodes,
}: {
  relations: SubgraphRelation[];
  onRelationsChange: (next: SubgraphRelation[]) => void;
  depth: number;
  onDepthChange: (depth: number) => void;
  onReset: () => void;
  onExport?: () => void;
  nodeCount: number;
  maxNodes: number;
}) {
  const toggle = (r: SubgraphRelation) =>
    onRelationsChange(relations.includes(r) ? relations.filter((x) => x !== r) : [...relations, r]);
  return (
    <div className="flex flex-wrap items-center gap-2" role="toolbar" aria-label="Graph options">
      {RELATIONS.map(({ value, label }) => (
        <Button
          key={value}
          size="sm"
          type="button"
          variant={relations.includes(value) ? 'default' : 'outline'}
          aria-pressed={relations.includes(value)}
          onClick={() => toggle(value)}
        >
          {label}
        </Button>
      ))}
      <Select
        aria-label="Depth"
        className="w-28"
        value={depth}
        onChange={(e) => onDepthChange(Number(e.target.value))}
      >
        {[1, 2, 3].map((d) => (
          <option key={d} value={d}>
            depth {d}
          </option>
        ))}
      </Select>
      <Button size="sm" type="button" variant="ghost" onClick={onReset}>
        Reset
      </Button>
      <Button size="sm" type="button" variant="outline" onClick={onExport} disabled={!onExport}>
        <Download aria-hidden />
        PNG
      </Button>
      <span className="ml-auto text-xs text-muted-foreground tabular-nums">
        {nodeCount} / {maxNodes} nodes
      </span>
    </div>
  );
}
