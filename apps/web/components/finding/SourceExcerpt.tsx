'use client';

import { useEffect, useState, type CSSProperties } from 'react';
import type { SourceExcerpt as SourceExcerptData } from '@/lib/api/pending';
import { highlight, type HighlightToken } from '@/lib/highlight';
import { languageForPath } from '@/lib/languages';
import { cn } from '@/lib/utils';

/** The API caps excerpts at 200 lines; the UI never shows more. */
export const MAX_EXCERPT_LINES = 200;

/**
 * A redacted source excerpt from the API with line numbers. It renders plain text first and
 * swaps in shiki tokens (highlighted server-side) when they arrive. Text is always rendered as
 * text, never as HTML.
 */
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
  const lines = excerpt.text.split('\n').slice(0, MAX_EXCERPT_LINES);
  const [tokens, setTokens] = useState<HighlightToken[][] | null>(null);
  const language = excerpt.language ?? languageForPath(excerpt.path);
  const text = lines.join('\n');

  useEffect(() => {
    let cancelled = false;
    highlight(text, language)
      .then((result) => {
        if (!cancelled) setTokens(result);
      })
      .catch(() => {
        // Plain text stays on screen.
      });
    return () => {
      cancelled = true;
    };
  }, [text, language]);

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
      <pre className="py-2 font-mono text-xs leading-5" data-testid="source-excerpt">
        {lines.map((line, i) => {
          const n = excerpt.start_line + i;
          const marked = highlightLines && n >= highlightLines[0] && n <= highlightLines[1];
          const lineTokens = tokens?.[i];
          return (
            <div
              key={n}
              className={cn('flex px-3', marked && 'bg-amber-200/30 dark:bg-amber-500/15')}
            >
              <span
                aria-hidden
                className="w-12 shrink-0 pr-3 text-right text-muted-foreground select-none"
              >
                {n}
              </span>
              <code className="whitespace-pre">
                {lineTokens
                  ? lineTokens.map((t, j) => (
                      <span
                        key={j}
                        className="shiki-token"
                        style={
                          { '--shiki-light': t.light, '--shiki-dark': t.dark } as CSSProperties
                        }
                      >
                        {t.content}
                      </span>
                    ))
                  : line}
              </code>
            </div>
          );
        })}
      </pre>
    </div>
  );
}
