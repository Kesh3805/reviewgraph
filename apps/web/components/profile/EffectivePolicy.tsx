import { Badge } from '@/components/ui/badge';
import type { EffectivePolicy as EffectivePolicyData, PolicySource } from '@/lib/api/pending';

/** PRD §65 precedence: explicit > documented > convention > generic. */
export const SOURCE_RANK: Record<PolicySource['kind'], number> = {
  explicit: 4,
  documented: 3,
  convention: 2,
  generic: 1,
};

export function describeSource(source: PolicySource): string {
  const id = source.id ? ` ${source.id}` : '';
  const extra =
    source.kind === 'convention' && source.confidence != null
      ? ` (confidence ${source.confidence.toFixed(2)}${source.samples != null ? `, ${source.samples} samples` : ''})`
      : '';
  return `${source.kind}${id}${extra}`;
}

/** The effective policy per topic, with the winning source and the overridden ones (POL-004). */
export function EffectivePolicy({ policies }: { policies: EffectivePolicyData[] }) {
  if (policies.length === 0) {
    return <p className="text-sm text-muted-foreground">No policy applies.</p>;
  }
  return (
    <ul aria-label="Effective policy" className="divide-y rounded-md border text-sm">
      {policies.map((p) => (
        <li key={p.topic} className="space-y-1 px-3 py-2" data-testid={`policy-${p.topic}`}>
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-xs">{p.topic}</span>
            <Badge variant="outline" className="uppercase">
              {p.decision.replace('_', ' ')}
            </Badge>
            {p.conflict && <Badge variant="warning">conflict</Badge>}
          </div>
          <p>
            <span className="text-muted-foreground">Winner:</span> {describeSource(p.winner)}
          </p>
          {p.overridden.length > 0 && (
            <p className="text-muted-foreground">
              Overridden:{' '}
              {[...p.overridden]
                .sort((a, b) => SOURCE_RANK[b.kind] - SOURCE_RANK[a.kind])
                .map((s) => (
                  <s key={`${s.kind}:${s.id ?? ''}`} className="mr-2">
                    {describeSource(s)}
                  </s>
                ))}
            </p>
          )}
        </li>
      ))}
    </ul>
  );
}
