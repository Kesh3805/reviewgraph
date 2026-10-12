import { Badge, type BadgeVariant } from '@/components/ui/badge';
import type { RiskAssessment } from '@/lib/api/pending';
import { formatNumber } from '@/lib/format';

const LEVEL_VARIANT: Record<RiskAssessment['level'], BadgeVariant> = {
  low: 'success',
  medium: 'warning',
  high: 'danger',
  critical: 'danger',
};

/** Risk level, score, signals and their effects (reviewers chosen, depth, budgets). */
export function RiskSummary({ risk }: { risk: RiskAssessment | null }) {
  if (!risk) return <p className="text-sm text-muted-foreground">No risk assessment.</p>;
  return (
    <div className="space-y-3 text-sm">
      <div className="flex items-center gap-3">
        <Badge variant={LEVEL_VARIANT[risk.level]} className="capitalize">
          {risk.level}
        </Badge>
        <span className="tabular-nums">score {risk.score.toFixed(2)}</span>
      </div>
      <ul aria-label="Risk signals" className="space-y-1">
        {risk.signals.map((s) => (
          <li key={s.name} className="flex justify-between gap-2">
            <span>
              <span className="font-mono text-xs">{s.name}</span>
              {s.detail && <span className="ml-2 text-muted-foreground">{s.detail}</span>}
            </span>
            <span className="text-muted-foreground tabular-nums">+{s.weight.toFixed(2)}</span>
          </li>
        ))}
      </ul>
      <dl className="grid grid-cols-2 gap-x-4 gap-y-1">
        <dt className="text-muted-foreground">Reviewers</dt>
        <dd className="capitalize">{risk.effects.reviewers.join(', ') || '-'}</dd>
        <dt className="text-muted-foreground">Depth</dt>
        <dd>{risk.effects.depth}</dd>
        <dt className="text-muted-foreground">Token budget</dt>
        <dd className="tabular-nums">
          {risk.effects.token_budget == null ? '-' : formatNumber(risk.effects.token_budget)}
        </dd>
        <dt className="text-muted-foreground">Model call budget</dt>
        <dd className="tabular-nums">{risk.effects.model_call_budget ?? '-'}</dd>
      </dl>
    </div>
  );
}
