import type { Severity } from '@/lib/api/pending';
import { SEVERITY_ORDER, SEVERITY_TOKENS } from '@/lib/severity';

/** Compact non-zero severity counts (`2 Critical · 1 High`). */
export function SeverityCounts({ counts }: { counts: Partial<Record<Severity, number>> }) {
  const present = SEVERITY_ORDER.filter((s) => (counts[s] ?? 0) > 0);
  if (present.length === 0) return <span className="text-muted-foreground">None</span>;
  return (
    <span className="flex flex-wrap gap-x-3 gap-y-1">
      {present.map((s) => (
        <span
          key={s}
          className={`flex items-center gap-1 ${SEVERITY_TOKENS[s].text}`}
          title={SEVERITY_TOKENS[s].label}
        >
          <span aria-hidden className={`size-2 rounded-full ${SEVERITY_TOKENS[s].dot}`} />
          <span className="tabular-nums">{counts[s]}</span>
          <span className="sr-only">{SEVERITY_TOKENS[s].label}</span>
        </span>
      ))}
    </span>
  );
}
