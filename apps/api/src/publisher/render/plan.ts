import { incCounter } from '../../common/metrics';
import { anchorFinding } from './anchor';
import type { DiffIndex } from './diff-index';
import { RenderError, renderInlineComment } from './inline-comment';
import type { InlineAnchor, RelocationReason, RenderableFinding } from './types';

export const DEFAULT_INLINE_CAP = 25;

export interface PlannedInline {
  finding: RenderableFinding;
  anchor: InlineAnchor;
  body: string;
}

export interface Relocated {
  finding: RenderableFinding;
  reason: RelocationReason;
}

export interface PublicationPlan {
  inline: PlannedInline[];
  /** Findings that appear in the summary instead. Nothing is ever dropped. */
  relocated: Relocated[];
}

/**
 * Splits findings (already in priority order, DED-004) into anchored inline comments and
 * summary relocations. Out-of-diff, over-cap and unrenderable findings are relocated, never
 * dropped (INV-015).
 */
export function planPublication(
  findings: readonly RenderableFinding[],
  diff: DiffIndex,
  opts: { inlineCap?: number } = {},
): PublicationPlan {
  const cap = opts.inlineCap ?? DEFAULT_INLINE_CAP;
  const plan: PublicationPlan = { inline: [], relocated: [] };
  const relocate = (finding: RenderableFinding, reason: RelocationReason): void => {
    incCounter('findings_relocated_to_summary_total', { reason });
    plan.relocated.push({ finding, reason });
  };

  for (const finding of findings) {
    const anchor = anchorFinding(finding, diff);
    if (anchor.kind === 'summary') {
      relocate(finding, anchor.reason);
      continue;
    }
    let body: string;
    try {
      body = renderInlineComment(finding);
    } catch (err) {
      if (err instanceof RenderError) {
        relocate(finding, 'render_error');
        continue;
      }
      throw err;
    }
    if (plan.inline.length >= cap) {
      relocate(finding, 'inline_cap');
      continue;
    }
    plan.inline.push({ finding, anchor, body });
  }
  return plan;
}
