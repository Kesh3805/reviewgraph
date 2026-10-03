import type { DiffIndex, Hunk } from './diff-index';
import type { Anchor, RenderableFinding } from './types';

type Side = 'LEFT' | 'RIGHT';

/** Numbers visible on one side of a hunk: new side = added + context, old side = deleted + context. */
function visible(hunk: Hunk, side: Side): { line: number; changed: boolean }[] {
  return hunk.lines.flatMap((l) => {
    const line = side === 'RIGHT' ? l.newLine : l.oldLine;
    return line === undefined ? [] : [{ line, changed: l.kind !== 'ctx' }];
  });
}

/**
 * Places a finding on the diff (GH-007).
 *
 * - `head` findings go on the RIGHT side against new-file numbers; `base` findings (deleted
 *   lines) go on the LEFT side against old-file numbers.
 * - A range wholly inside one hunk becomes a multi-line comment (`line = end`,
 *   `start_line = start`). A range that only partly overlaps, or spans hunks, falls back to a
 *   single line: the first changed visible line in the range, else the first visible line.
 * - Anything else (no location, file not in the diff, no visible line) goes to the summary.
 */
export function anchorFinding(finding: RenderableFinding, diff: DiffIndex): Anchor {
  const loc = finding.location;
  const outside: Anchor = { kind: 'summary', reason: 'outside_diff' };
  if (!loc || loc.startLine < 1 || loc.endLine < loc.startLine) return outside;
  const side: Side = loc.side === 'base' ? 'LEFT' : 'RIGHT';

  const hunks = diff.hunks(loc.path);
  for (const hunk of hunks) {
    const lines = visible(hunk, side);
    if (lines.length === 0) continue;
    const first = lines[0]!.line;
    const last = lines[lines.length - 1]!.line;
    if (loc.startLine >= first && loc.endLine <= last) {
      return loc.startLine < loc.endLine
        ? {
            kind: 'inline',
            path: loc.path,
            line: loc.endLine,
            side,
            startLine: loc.startLine,
            startSide: side,
          }
        : { kind: 'inline', path: loc.path, line: loc.endLine, side };
    }
  }

  const inRange = hunks
    .flatMap((h) => visible(h, side))
    .filter((l) => l.line >= loc.startLine && l.line <= loc.endLine);
  const pick = inRange.find((l) => l.changed) ?? inRange[0];
  if (!pick) return outside;
  return { kind: 'inline', path: loc.path, line: pick.line, side };
}
