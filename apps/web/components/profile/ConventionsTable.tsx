import { Badge } from '@/components/ui/badge';
import type { Convention } from '@/lib/api/pending';
import { formatPercent } from '@/lib/format';

/** PROF-003 / target-arch §3.10 thresholds. */
export const ENFORCEABLE_MIN_CONFIDENCE = 0.9;
export const ENFORCEABLE_MIN_SAMPLES = 10;

export function isEnforceable(c: Convention): boolean {
  return (
    c.enforceable ??
    (c.confidence >= ENFORCEABLE_MIN_CONFIDENCE && c.samples >= ENFORCEABLE_MIN_SAMPLES)
  );
}

/** Rule, scope, samples, violations, consistency, confidence, enforceable badge, exceptions. */
export function ConventionsTable({ conventions }: { conventions: Convention[] }) {
  if (conventions.length === 0) {
    return <p className="text-sm text-muted-foreground">No conventions were inferred.</p>;
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm" aria-label="Conventions">
        <thead className="text-xs text-muted-foreground uppercase">
          <tr>
            <th className="py-2 pr-4 font-medium">Rule</th>
            <th className="py-2 pr-4 font-medium">Scope</th>
            <th className="py-2 pr-4 text-right font-medium">Samples</th>
            <th className="py-2 pr-4 text-right font-medium">Violations</th>
            <th className="py-2 pr-4 text-right font-medium">Consistency</th>
            <th className="py-2 pr-4 text-right font-medium">Confidence</th>
            <th className="py-2 pr-4 font-medium">Status</th>
            <th className="py-2 font-medium">Exceptions</th>
          </tr>
        </thead>
        <tbody>
          {conventions.map((c) => (
            <tr key={c.id} className="border-t align-top" data-testid={`convention-${c.id}`}>
              <td className="py-2 pr-4">
                <span className="font-mono text-xs">{c.id}</span>
                <p className="text-muted-foreground">{c.rule}</p>
              </td>
              <td className="py-2 pr-4 font-mono text-xs">{c.scope}</td>
              <td className="py-2 pr-4 text-right tabular-nums">{c.samples}</td>
              <td className="py-2 pr-4 text-right tabular-nums">{c.violations}</td>
              <td className="py-2 pr-4 text-right tabular-nums">{formatPercent(c.consistency)}</td>
              <td className="py-2 pr-4 text-right tabular-nums">{c.confidence.toFixed(2)}</td>
              <td className="py-2 pr-4">
                {isEnforceable(c) ? (
                  <Badge variant="success">Enforceable</Badge>
                ) : (
                  <Badge variant="muted">Informational</Badge>
                )}
                <span className="ml-1 text-xs text-muted-foreground">{c.source}</span>
              </td>
              <td className="py-2 text-xs">
                {c.exceptions.length === 0 ? (
                  <span className="text-muted-foreground">-</span>
                ) : (
                  <ul>
                    {c.exceptions.map((e) => (
                      <li key={e.scope}>
                        <span className="font-mono">{e.scope}</span>
                        {e.reason && <span className="text-muted-foreground"> · {e.reason}</span>}
                      </li>
                    ))}
                  </ul>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
