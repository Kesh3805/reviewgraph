import { Badge } from '@/components/ui/badge';
import type { ReviewState } from '@/lib/api/pending';
import { runStateDisplay } from '@/lib/run-state';

/** Maps every PIPE run state (including SUPERSEDED, CANCELLED and FAILED_*) to a badge. */
export function RunStateBadge({ state, degraded }: { state: ReviewState; degraded?: boolean }) {
  const { label, variant } = runStateDisplay(state, degraded);
  return (
    <Badge variant={variant} data-state={state}>
      {label}
    </Badge>
  );
}
