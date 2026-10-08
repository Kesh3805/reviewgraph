'use server';

import { bundledLanguages, codeToTokens } from 'shiki';

/** One highlighted token: the text plus its light and dark theme colours. */
export interface HighlightToken {
  content: string;
  light?: string;
  dark?: string;
}

/** Excerpts are capped at 200 lines by the API; refuse anything far larger. */
const MAX_CHARS = 200 * 400;

/**
 * Server-side syntax highlighting with shiki. The text is the already redacted excerpt from the
 * API; the result is plain tokens (no HTML), so the client renders it as text.
 */
export async function highlight(
  text: string,
  language: string | null,
): Promise<HighlightToken[][] | null> {
  if (typeof text !== 'string' || text.length > MAX_CHARS) return null;
  const lang = language && language in bundledLanguages ? language : null;
  if (!lang) return null;
  const { tokens } = await codeToTokens(text, {
    lang: lang as keyof typeof bundledLanguages,
    themes: { light: 'github-light', dark: 'github-dark' },
    defaultColor: false,
  });
  return tokens.map((line) =>
    line.map((token) => {
      const style = (token.htmlStyle ?? {}) as Record<string, string>;
      return {
        content: token.content,
        light: style['--shiki-light'] ?? token.color,
        dark: style['--shiki-dark'] ?? token.color,
      };
    }),
  );
}
