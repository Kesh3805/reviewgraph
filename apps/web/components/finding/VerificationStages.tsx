import { Badge, type BadgeVariant } from '@/components/ui/badge';
import type { FindingTrace } from '@/lib/api/pending';

const OUTCOME: Record<FindingTrace['verification'][number]['outcome'], BadgeVariant> = {
  passed: 'success',
  failed: 'danger',
  inconclusive: 'warning',
  not_executed: 'muted',
};

/** Verification stages with their outcome and evidence items (references, never code). */
export function VerificationStages({ stages }: { stages: FindingTrace['verification'] }) {
  if (stages.length === 0) {
    return <p className="text-sm text-muted-foreground">No verification stage recorded.</p>;
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead className="text-xs text-muted-foreground uppercase">
          <tr>
            <th className="py-2 pr-4 font-medium">Stage</th>
            <th className="py-2 pr-4 font-medium">Outcome</th>
            <th className="py-2 font-medium">Evidence</th>
          </tr>
        </thead>
        <tbody>
          {stages.map((s) => (
            <tr key={s.stage} className="border-t align-top">
              <td className="py-2 pr-4">{s.stage}</td>
              <td className="py-2 pr-4">
                <Badge variant={OUTCOME[s.outcome]}>
                  {s.outcome === 'not_executed' ? 'NOT EXECUTED' : s.outcome}
                </Badge>
              </td>
              <td className="py-2">
                {s.evidence.length === 0 ? (
                  <span className="text-muted-foreground">-</span>
                ) : (
                  <ul className="space-y-0.5">
                    {s.evidence.map((e, i) => (
                      <li key={i}>
                        <span className="text-xs text-muted-foreground">{e.kind}</span> {e.summary}
                        {e.path && (
                          <span className="ml-1 font-mono text-xs text-muted-foreground">
                            {e.path}
                            {e.start_line ? `:${e.start_line}` : ''}
                          </span>
                        )}
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
