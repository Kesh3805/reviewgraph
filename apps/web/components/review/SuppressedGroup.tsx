'use client';

import { ChevronRight } from 'lucide-react';
import { useState } from 'react';
import type { FindingGroups } from '@/lib/findings';
import { FindingCard } from './FindingCard';

/** "Suppressed (N)", collapsed by default, grouped by suppression reason. */
export function SuppressedGroup({ groups }: { groups: FindingGroups['suppressed'] }) {
  const [open, setOpen] = useState(false);
  const total = groups.reduce((n, g) => n + g.items.length, 0);
  if (total === 0) return null;
  return (
    <section aria-label="Suppressed findings" className="rounded-md border">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm font-medium"
      >
        <ChevronRight
          aria-hidden
          className={`size-4 transition-transform ${open ? 'rotate-90' : ''}`}
        />
        Suppressed ({total})
      </button>
      {open && (
        <div className="space-y-3 border-t p-3">
          {groups.map((group) => (
            <div key={group.state} data-testid={`suppressed-${group.state}`}>
              <h4 className="text-xs font-medium text-muted-foreground uppercase">
                {group.label} ({group.items.length})
              </h4>
              <ul className="divide-y">
                {group.items.map((f) => (
                  <FindingCard key={f.id} finding={f} />
                ))}
              </ul>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
