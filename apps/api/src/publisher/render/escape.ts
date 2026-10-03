/**
 * Escapes untrusted text (finding titles, model-authored claims) for GitHub markdown.
 * Markdown punctuation is backslash-escaped; `<`, `>`, `&` and `@` become entities, so text can
 * neither inject HTML, close the hidden `<!-- -->` markers, nor ping users. Newlines collapse to
 * spaces for single-line contexts.
 */
export function escapeMarkdown(text: string): string {
  return text
    .replace(/\r\n?|\n/g, ' ')
    .replace(/[&<>@]/g, (c) => `&#${c.charCodeAt(0)};`)
    .replace(/[\\`*_[\]~|#]/g, (c) => `\\${c}`)
    .trim();
}

/** Multi-paragraph variant: keeps blank-line paragraph breaks, escapes each paragraph. */
export function escapeParagraphs(text: string): string {
  return text
    .split(/\r?\n\s*\r?\n/)
    .map((p) => escapeMarkdown(p))
    .filter(Boolean)
    .join('\n\n');
}

/** Text safe inside a fenced ```text block: no fence-breaking backticks, single line. */
export function fenceSafe(text: string): string {
  return text
    .replace(/`/g, "'")
    .replace(/\r\n?|\n/g, ' ')
    .trim();
}

/** Marker values are restricted to a conservative alphabet so they cannot close the comment. */
export function markerSafe(value: string): string {
  return value.replace(/[^A-Za-z0-9._:-]/g, '_');
}
