import { escapeMarkdown, escapeParagraphs, fenceSafe, markerSafe } from './escape';
import { severityEmoji, severityLabel } from './severity';
import type { RenderableFinding } from './types';

/** A finding that lacks a required explanation field; it is relocated to the summary. */
export class RenderError extends Error {
  constructor(
    readonly findingId: string,
    readonly missing: string[],
  ) {
    super(`finding ${findingId} cannot be rendered: missing ${missing.join(', ')}`);
    this.name = 'RenderError';
  }
}

/** PRD section 58 hedging phrases; rejected upstream (VER stage 7), asserted here in development. */
export const BANNED_HEDGES = [
  'this potentially might',
  'consider whether',
  'maybe this could',
] as const;

export const LATENT_NOTE =
  '_Latent:_ no current caller reaches this. It is reported because the defect is real, not because it blocks this PR.';

const REQUIRED: (keyof RenderableFinding)[] = [
  'title',
  'whatChanged',
  'whyRisky',
  'behaviorResult',
  'correctiveDirection',
];

/**
 * Renders the PRD section 58/59 comment: severity line, what changed and why it is risky,
 * the resulting behavior, the evidence path, the corrective direction, a footer and the hidden
 * finding marker. Pure and deterministic.
 */
export function renderInlineComment(f: RenderableFinding): string {
  const missing = REQUIRED.filter((k) => typeof f[k] !== 'string' || !(f[k] as string).trim());
  if (missing.length > 0) throw new RenderError(f.id, missing);

  const parts: string[] = [];
  parts.push(
    `${severityEmoji(f.severity)} **${severityLabel(f.severity)} — ${escapeMarkdown(f.title)}**`,
  );
  parts.push(`${escapeMarkdown(f.whatChanged)}  ${escapeMarkdown(f.whyRisky)}`);
  parts.push(escapeParagraphs(f.behaviorResult));
  parts.push(renderEvidence(f));
  parts.push(escapeParagraphs(f.correctiveDirection));
  if (f.latent) parts.push(LATENT_NOTE);
  parts.push(
    `<sub>${escapeMarkdown(f.shortId)} · ${escapeMarkdown(f.reviewer)} · confidence ${f.confidence.toFixed(2)}</sub>\n` +
      `<!-- reviewgraph:finding=${markerSafe(f.fingerprint)} run=${markerSafe(f.reviewRunId)} -->`,
  );
  const body = parts.filter(Boolean).join('\n\n');

  if (process.env.NODE_ENV !== 'production') {
    const lower = body.toLowerCase();
    const hedge = BANNED_HEDGES.find((h) => lower.includes(h));
    if (hedge) throw new Error(`hedging phrase "${hedge}" reached the renderer (finding ${f.id})`);
  }
  return body;
}

function renderEvidence(f: RenderableFinding): string {
  const path = (f.evidencePath ?? []).map(fenceSafe).filter(Boolean);
  if (path.length > 0) {
    const lines = path.map((p, i) => (i === 0 ? p : `  → ${p}`));
    return `**Evidence**\n\`\`\`text\n${lines.join('\n')}\n\`\`\``;
  }
  const items = (f.evidenceItems ?? []).slice(0, 5);
  if (items.length === 0) return '';
  const cited = items.map((e) => {
    const where = e.path ? ` (\`${fenceSafe(e.path)}${e.line ? `:${e.line}` : ''}\`)` : '';
    return `- ${escapeMarkdown(e.claim)}${where}`;
  });
  return `**Evidence**\n${cited.join('\n')}`;
}
