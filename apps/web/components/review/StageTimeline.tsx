import { Badge, type BadgeVariant } from '@/components/ui/badge';
import type { ErrorClass, StageTiming } from '@/lib/api/pending';
import { formatDuration } from '@/lib/format';

const STAGE_VARIANT: Record<StageTiming['state'], BadgeVariant> = {
  pending: 'muted',
  running: 'info',
  succeeded: 'success',
  failed: 'danger',
  skipped: 'muted',
};

/** Stages in order with durations; bar widths are relative to the slowest stage. */
export function StageTimeline({
  stages,
  failure,
}: {
  stages: StageTiming[];
  failure: { stage: string; error_class: ErrorClass } | null;
}) {
  const max = Math.max(1, ...stages.map((s) => s.duration_ms ?? 0));
  return (
    <div className="space-y-2">
      {failure && (
        <p role="alert" className="text-sm text-destructive">
          Failed at stage <span className="font-medium">{failure.stage}</span> (
          {failure.error_class}).
        </p>
      )}
      {stages.length === 0 ? (
        <p className="text-sm text-muted-foreground">No stage has started yet.</p>
      ) : (
        <ol aria-label="Stage timeline" className="space-y-1 text-sm">
          {stages.map((stage) => (
            <li key={stage.name} className="grid grid-cols-[10rem_1fr_5rem] items-center gap-3">
              <span className="truncate">{stage.name}</span>
              <span className="flex items-center gap-2">
                <span
                  aria-hidden
                  className="h-2 rounded bg-primary/40"
                  style={{ width: `${Math.max(2, ((stage.duration_ms ?? 0) / max) * 100)}%` }}
                />
                <Badge variant={STAGE_VARIANT[stage.state]}>{stage.state}</Badge>
              </span>
              <span className="text-right text-muted-foreground tabular-nums">
                {formatDuration(stage.duration_ms)}
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
