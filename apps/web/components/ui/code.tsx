'use client';

import { useEffect, useState, type CSSProperties } from 'react';
import { highlight, type HighlightToken } from '@/lib/highlight';
import { cn } from '@/lib/utils';

/**
 * Code with line numbers. Plain text renders first; shiki tokens (highlighted server-side)
 * replace it when they arrive. Text is always rendered as text, never as HTML.
 */
export function HighlightedCode({
  text,
  language,
  startLine = 1,
  highlightLines,
  maxLines,
  testId,
}: {
  text: string;
  language: string | null;
  startLine?: number;
  /** Inclusive line range to emphasise. */
  highlightLines?: [number, number];
  maxLines?: number;
  testId?: string;
}) {
  const lines = text.split('\n').slice(0, maxLines);
  const shown = lines.join('\n');
  const [tokens, setTokens] = useState<HighlightToken[][] | null>(null);

  useEffect(() => {
    let cancelled = false;
    highlight(shown, language)
      .then((result) => {
        if (!cancelled) setTokens(result);
      })
      .catch(() => {
        // Plain text stays on screen.
      });
    return () => {
      cancelled = true;
    };
  }, [shown, language]);

  return (
    <pre className="py-2 font-mono text-xs leading-5" data-testid={testId}>
      {lines.map((line, i) => {
        const n = startLine + i;
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
                      style={{ '--shiki-light': t.light, '--shiki-dark': t.dark } as CSSProperties}
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
  );
}
