import type { ConfidenceComponent, ConfidenceTerm } from '@/lib/api/pending';

export const TERM_ORDER: ConfidenceTerm[] = [
  'anchor',
  'deterministic',
  'graph',
  'repo',
  'reproduction',
  'agreement',
  'contradiction',
  'uncertainty',
];

/** ADR-011: `clamp(0, 1, Σ weight × value)`. */
export function computeConfidence(components: ConfidenceComponent[]): number {
  const sum = components.reduce((acc, c) => acc + c.weight * c.value, 0);
  return Math.min(1, Math.max(0, sum));
}

/**
 * The computed confidence with one bar per ADR-011 term (weight × value; penalties in red),
 * using the weights of the run's `verification_version`.
 */
export function ConfidenceBreakdown({
  confidence,
  components,
  verificationVersion,
}: {
  confidence: number;
  components: ConfidenceComponent[];
  verificationVersion: string;
}) {
  const ordered = [...components].sort(
    (a, b) => TERM_ORDER.indexOf(a.term) - TERM_ORDER.indexOf(b.term),
  );
  const computed = computeConfidence(components);
  const mismatch = Math.abs(computed - confidence) > 0.005;

  return (
    <div className="space-y-2 text-sm">
      <p>
        Confidence <span className="font-semibold tabular-nums">{confidence.toFixed(2)}</span>
        <span className="ml-2 text-xs text-muted-foreground">
          components sum to{' '}
          <span data-testid="confidence-sum" className="tabular-nums">
            {computed.toFixed(2)}
          </span>{' '}
          · weights {verificationVersion}
        </span>
      </p>
      {mismatch && (
        <p role="alert" className="text-xs text-destructive">
          The components do not add up to the stored confidence.
        </p>
      )}
      <ul aria-label="Confidence components" className="space-y-1">
        {ordered.map((c) => {
          const contribution = c.weight * c.value;
          return (
            <li key={c.term} className="grid grid-cols-[7rem_1fr_7rem] items-center gap-2 text-xs">
              <span className="capitalize">{c.term}</span>
              <span className="h-2 rounded bg-muted">
                <span
                  className={`block h-2 rounded ${contribution < 0 ? 'bg-red-500' : 'bg-primary/60'}`}
                  style={{ width: `${Math.min(100, Math.abs(contribution) * 100)}%` }}
                />
              </span>
              <span className="text-right text-muted-foreground tabular-nums">
                {c.weight.toFixed(2)} × {c.value.toFixed(2)} = {contribution >= 0 ? '+' : ''}
                {contribution.toFixed(3)}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
