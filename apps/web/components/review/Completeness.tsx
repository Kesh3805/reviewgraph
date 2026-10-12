import { Badge } from '@/components/ui/badge';
import type { Completeness as CompletenessData, ReviewerRun } from '@/lib/api/pending';
import { formatDuration } from '@/lib/format';

/**
 * Planned, succeeded, failed and not-executed reviewers and checks. A check that did not run is
 * a grey "NOT EXECUTED — reason" chip: it is never shown as green.
 */
export function Completeness({
  completeness,
  reviewerRuns,
}: {
  completeness: CompletenessData;
  reviewerRuns: ReviewerRun[];
}) {
  const succeeded = new Set(completeness.reviewers_succeeded);
  const failed = new Map(completeness.reviewers_failed.map((f) => [f.reviewer, f.reason]));
  const runs = new Map(reviewerRuns.map((r) => [r.reviewer, r]));

  return (
    <div className="space-y-3 text-sm">
      <p className="text-muted-foreground">
        {completeness.reviewers_succeeded.length} of {completeness.reviewers_planned.length} planned
        reviewers succeeded
        {completeness.reviewers_failed.length > 0 &&
          `, ${completeness.reviewers_failed.length} failed`}
        .
      </p>
      <ul aria-label="Reviewers" className="divide-y rounded-md border">
        {completeness.reviewers_planned.map((reviewer) => {
          const run = runs.get(reviewer);
          return (
            <li
              key={reviewer}
              data-testid={`reviewer-${reviewer}`}
              className="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
            >
              <span>
                <span className="font-medium capitalize">{reviewer}</span>
                {run && (
                  <span className="ml-2 text-xs text-muted-foreground">
                    {reviewer}:{run.version} · {formatDuration(run.duration_ms)} · {run.findings}{' '}
                    findings
                  </span>
                )}
              </span>
              {succeeded.has(reviewer) ? (
                <Badge variant="success">Succeeded</Badge>
              ) : failed.has(reviewer) ? (
                <Badge variant="danger" title={failed.get(reviewer)}>
                  Failed — {failed.get(reviewer)}
                </Badge>
              ) : (
                <Badge variant="muted">{run?.state ?? 'Pending'}</Badge>
              )}
            </li>
          );
        })}
      </ul>
      {completeness.not_executed.length > 0 && (
        <div className="flex flex-wrap gap-2" aria-label="Not executed checks">
          {completeness.not_executed.map((n) => (
            <Badge key={n.check} variant="muted" data-testid="not-executed" className="uppercase">
              Not executed — {n.check}: {n.reason}
            </Badge>
          ))}
        </div>
      )}
    </div>
  );
}
