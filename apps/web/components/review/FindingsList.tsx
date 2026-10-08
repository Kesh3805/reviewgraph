'use client';

import type { FindingSummary } from '@/lib/api/pending';
import { groupFindings } from '@/lib/findings';
import { FindingCard } from './FindingCard';
import { SuppressedGroup } from './SuppressedGroup';

/**
 * Published findings first, then relocated (outside the diff) findings, then a collapsible
 * "Suppressed (N)" group by reason.
 */
export function FindingsList({ findings }: { findings: FindingSummary[] }) {
  const groups = groupFindings(findings);
  return (
    <div className="space-y-4">
      <section aria-label="Published findings">
        <h3 className="mb-1 text-sm font-medium">Published ({groups.published.length})</h3>
        {groups.published.length === 0 ? (
          <p className="text-sm text-muted-foreground">No published findings.</p>
        ) : (
          <ul className="divide-y rounded-md border">
            {groups.published.map((f) => (
              <FindingCard key={f.id} finding={f} />
            ))}
          </ul>
        )}
      </section>

      {groups.relocated.length > 0 && (
        <section aria-label="Relocated findings">
          <h3 className="mb-1 text-sm font-medium">Outside the diff ({groups.relocated.length})</h3>
          <p className="mb-1 text-xs text-muted-foreground">
            Anchored on lines the pull request did not change; published in the summary.
          </p>
          <ul className="divide-y rounded-md border">
            {groups.relocated.map((f) => (
              <FindingCard key={f.id} finding={f} />
            ))}
          </ul>
        </section>
      )}

      {groups.pending.length > 0 && (
        <section aria-label="Pending findings">
          <h3 className="mb-1 text-sm font-medium">In verification ({groups.pending.length})</h3>
          <ul className="divide-y rounded-md border">
            {groups.pending.map((f) => (
              <FindingCard key={f.id} finding={f} />
            ))}
          </ul>
        </section>
      )}

      <SuppressedGroup groups={groups.suppressed} />
    </div>
  );
}
