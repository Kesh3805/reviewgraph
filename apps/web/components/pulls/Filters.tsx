'use client';

import { Label, Select } from '@/components/ui/input';
import type { Repository, Severity } from '@/lib/api/pending';
import type { PullFilters } from '@/lib/pull-filters';
import { SEVERITY_ORDER, SEVERITY_TOKENS } from '@/lib/severity';

/** Repository, state, has-findings and severity filters. Any change resets the cursor. */
export function Filters({
  filters,
  onChange,
  repositories,
}: {
  filters: PullFilters;
  onChange: (next: PullFilters) => void;
  /** Omitted on the repository tab, where the repository is fixed. */
  repositories?: Repository[];
}) {
  const set = (patch: Partial<PullFilters>) =>
    onChange({ ...filters, ...patch, cursor: undefined });

  return (
    <div role="search" aria-label="Pull request filters" className="flex flex-wrap items-end gap-3">
      {repositories && (
        <div className="space-y-1">
          <Label htmlFor="filter-repository">Repository</Label>
          <Select
            id="filter-repository"
            className="w-56"
            value={filters.repository ?? ''}
            onChange={(e) => set({ repository: e.target.value || undefined })}
          >
            <option value="">All repositories</option>
            {repositories.map((r) => (
              <option key={r.id} value={r.id}>
                {r.full_name}
              </option>
            ))}
          </Select>
        </div>
      )}
      <div className="space-y-1">
        <Label htmlFor="filter-state">State</Label>
        <Select
          id="filter-state"
          className="w-32"
          value={filters.state ?? ''}
          onChange={(e) =>
            set({ state: (e.target.value || undefined) as PullFilters['state'] | undefined })
          }
        >
          <option value="">Any</option>
          <option value="open">Open</option>
          <option value="closed">Closed</option>
        </Select>
      </div>
      <div className="space-y-1">
        <Label htmlFor="filter-findings">Findings</Label>
        <Select
          id="filter-findings"
          className="w-40"
          value={filters.hasFindings === undefined ? '' : String(filters.hasFindings)}
          onChange={(e) =>
            set({ hasFindings: e.target.value === '' ? undefined : e.target.value === 'true' })
          }
        >
          <option value="">Any</option>
          <option value="true">Has findings</option>
          <option value="false">No findings</option>
        </Select>
      </div>
      <div className="space-y-1">
        <Label htmlFor="filter-severity">Severity</Label>
        <Select
          id="filter-severity"
          className="w-36"
          value={filters.severity ?? ''}
          onChange={(e) => set({ severity: (e.target.value || undefined) as Severity | undefined })}
        >
          <option value="">Any</option>
          {SEVERITY_ORDER.map((s) => (
            <option key={s} value={s}>
              {SEVERITY_TOKENS[s].label}
            </option>
          ))}
        </Select>
      </div>
    </div>
  );
}
