'use client';

import { Input, Select } from '@/components/ui/input';
import type { SuppressionKind } from '@/lib/api/pending';

export interface SuppressState {
  enabled: boolean;
  kind: SuppressionKind;
  reason: string;
}

export const DEFAULT_SUPPRESS: SuppressState = { enabled: false, kind: 'fingerprint', reason: '' };

/**
 * "Suppress future occurrences" for Intentional / Not relevant. Rendered for maintainers only;
 * the API enforces the role either way.
 */
export function SuppressOption({
  value,
  onChange,
  disabled,
}: {
  value: SuppressState;
  onChange: (next: SuppressState) => void;
  disabled?: boolean;
}) {
  return (
    <fieldset className="space-y-2 rounded-md border p-2" disabled={disabled}>
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="size-4"
          checked={value.enabled}
          onChange={(e) => onChange({ ...value, enabled: e.target.checked })}
        />
        Suppress future occurrences
      </label>
      {value.enabled && (
        <div className="flex flex-wrap gap-2">
          <Select
            aria-label="Suppress by"
            className="w-36"
            value={value.kind}
            onChange={(e) => onChange({ ...value, kind: e.target.value as SuppressionKind })}
          >
            <option value="fingerprint">This finding</option>
            <option value="symbol">This symbol</option>
            <option value="path">This path</option>
          </Select>
          <Input
            aria-label="Suppression reason"
            className="min-w-48 flex-1"
            placeholder="Reason"
            maxLength={500}
            value={value.reason}
            onChange={(e) => onChange({ ...value, reason: e.target.value })}
          />
        </div>
      )}
    </fieldset>
  );
}
