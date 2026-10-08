'use client';

import { HighlightedCode } from '@/components/ui/code';
import type { SourceExcerpt as SourceExcerptData } from '@/lib/api/pending';
import { languageForPath } from '@/lib/languages';
import { cn } from '@/lib/utils';

/** The API caps excerpts at 200 lines; the UI never shows more. */
export const MAX_EXCERPT_LINES = 200;

/** A redacted source excerpt from the API, with line numbers and server-side highlighting. */
export function SourceExcerpt({
  excerpt,
  highlightLines,
  className,
}: {
  excerpt: SourceExcerptData;
  /** Inclusive line range to emphasise (the finding anchor). */
  highlightLines?: [number, number];
  className?: string;
}) {
  return (
    <div className={cn('overflow-x-auto rounded-md border bg-muted/30', className)}>
      <div className="flex items-center justify-between border-b px-3 py-1 text-xs text-muted-foreground">
        <span className="font-mono">
          {excerpt.path}:{excerpt.start_line}-{excerpt.end_line}
        </span>
        <span>
          {excerpt.redacted && 'redacted · '}
          {excerpt.truncated && 'truncated · '}
          {excerpt.snapshot_id.slice(0, 8)}
        </span>
      </div>
      <HighlightedCode
        text={excerpt.text}
        language={excerpt.language ?? languageForPath(excerpt.path)}
        startLine={excerpt.start_line}
        highlightLines={highlightLines}
        maxLines={MAX_EXCERPT_LINES}
        testId="source-excerpt"
      />
    </div>
  );
}
