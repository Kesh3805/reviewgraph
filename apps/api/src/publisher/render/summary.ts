import { escapeMarkdown, fenceSafe, markerSafe } from './escape';
import { SEVERITY_ORDER, severityEmoji, severityLabel } from './severity';
import type { RelocationReason, Severity } from './types';

export const NO_MERGE_FOOTER = 'No merge performed. ReviewGraph never approves or merges.';
export const MAX_RISK_AREAS = 5;

export type SuppressionReason =
  'low_confidence' | 'duplicate' | 'pre_existing' | 'not_actionable' | 'policy';

const SUPPRESSION_LABEL: Record<SuppressionReason, string> = {
  low_confidence: 'low-confidence candidates',
  duplicate: 'duplicates',
  pre_existing: 'pre-existing issues',
  not_actionable: 'not actionable',
  policy: 'suppressed by policy',
};

export type CheckStatus = 'passed' | 'failed' | 'not_executed';

export interface RelocatedFindingSummary {
  severity: Severity;
  title: string;
  path?: string;
  line?: number;
  reason: RelocationReason;
}

/**
 * Everything the summary shows. Counts are computed in SQL (candidate_findings and findings
 * grouped by lifecycle state) and passed in; the renderer never derives them from model output.
 */
export interface SummaryInput {
  runId: string;
  headSha: string;
  /** Run finished with failed or missing reviewers. */
  degraded: boolean;
  changed: { files: number; behavioralSymbols: number; apiContracts: number };
  /** From the RiskAssessment signals; undefined renders "unavailable". */
  riskAreas?: string[];
  /** Published findings (inline and relocated) by severity. */
  findingsBySeverity: Partial<Record<Severity, number>>;
  verified: number;
  candidates: number;
  suppressed: Partial<Record<SuppressionReason, number>>;
  /** Findings relocated from inline comments (out of diff, over the cap, render error). */
  outsideDiff: RelocatedFindingSummary[];
  coverage: {
    reviewersRun: string[];
    reviewersNotRun: { reviewer: string; reason: string }[];
    unreviewedClusters: string[];
    checks: { name: string; status: CheckStatus; reason?: string }[];
  };
  /** POL-002 notice when the repository policy changed in this PR. */
  policyChange?: string;
  /**
   * Findings of an earlier review that are no longer reported although the code they point at
   * did not change (GH-011 "unknown"): their threads stay open and are listed here.
   */
  previouslyReported?: PreviouslyReportedFinding[];
}

export interface PreviouslyReportedFinding {
  severity: Severity;
  title: string;
  path?: string;
  line?: number;
}

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`;

/** Renders the PRD section 61 review summary. Pure, deterministic, never throws on content. */
export function renderSummary(s: SummaryInput): string {
  const out: string[] = ['## ReviewGraph'];

  if (s.degraded) out.push('> Review completed with reduced coverage.');

  out.push(
    [
      '**Changed**',
      `- ${plural(s.changed.files, 'file', 'files')}`,
      `- ${plural(s.changed.behavioralSymbols, 'behavioral symbol', 'behavioral symbols')}`,
      `- ${plural(s.changed.apiContracts, 'API contract', 'API contracts')}`,
    ].join('\n'),
  );

  if (s.riskAreas === undefined) {
    out.push('**Risk areas**\nRisk areas: unavailable');
  } else {
    const areas = s.riskAreas.slice(0, MAX_RISK_AREAS).map((a) => `- ${escapeMarkdown(a)}`);
    out.push(`**Risk areas**\n${areas.length > 0 ? areas.join('\n') : '- none identified'}`);
  }

  const severityLines = SEVERITY_ORDER.filter((sev) => (s.findingsBySeverity[sev] ?? 0) > 0).map(
    (sev) => `- ${s.findingsBySeverity[sev]} ${sev}`,
  );
  out.push(
    `**Findings**\n${severityLines.length > 0 ? severityLines.join('\n') : 'No verified findings.'}`,
  );

  out.push(`**Verified**\n${s.verified} / ${s.candidates} candidate findings`);

  const suppressed = (Object.keys(SUPPRESSION_LABEL) as SuppressionReason[])
    .filter((r) => (s.suppressed[r] ?? 0) > 0)
    .map((r) => `- ${s.suppressed[r]} ${SUPPRESSION_LABEL[r]}`);
  out.push(`**Suppressed**\n${suppressed.length > 0 ? suppressed.join('\n') : '- none'}`);

  if (s.outsideDiff.length > 0) {
    const items = s.outsideDiff.map((f) => {
      const where = f.path ? ` (\`${fenceSafe(f.path)}${f.line ? `:${f.line}` : ''}\`)` : '';
      return `- ${severityEmoji(f.severity)} **${severityLabel(f.severity)}** — ${escapeMarkdown(f.title)}${where}`;
    });
    out.push(
      `**Findings outside the diff**\nThese could not be placed as inline comments on this diff.\n${items.join('\n')}`,
    );
  }

  if (s.previouslyReported && s.previouslyReported.length > 0) {
    const items = s.previouslyReported.map((f) => {
      const where = f.path ? ` (\`${fenceSafe(f.path)}${f.line ? `:${f.line}` : ''}\`)` : '';
      return `- ${severityEmoji(f.severity)} **${severityLabel(f.severity)}** — ${escapeMarkdown(f.title)}${where}`;
    });
    out.push(
      `**Previously reported**\nNot reported on this head, but the code they point at did not change; their threads stay open.\n${items.join('\n')}`,
    );
  }

  out.push(renderCoverage(s.coverage));

  if (s.policyChange?.trim()) {
    out.push(`> **Policy change:** ${escapeMarkdown(s.policyChange)}`);
  }

  out.push(NO_MERGE_FOOTER);
  out.push(`<!-- reviewgraph:run=${markerSafe(s.runId)} head=${markerSafe(s.headSha)} -->`);
  return out.join('\n\n');
}

function renderCoverage(c: SummaryInput['coverage']): string {
  const lines = ['**Coverage**'];
  lines.push(
    `- Reviewers run: ${c.reviewersRun.length > 0 ? c.reviewersRun.map(escapeMarkdown).join(', ') : 'none'}`,
  );
  for (const r of c.reviewersNotRun) {
    lines.push(`- Not run: ${escapeMarkdown(r.reviewer)} — ${escapeMarkdown(r.reason)}`);
  }
  if (c.unreviewedClusters.length > 0) {
    lines.push(`- Unreviewed clusters: ${c.unreviewedClusters.map(escapeMarkdown).join(', ')}`);
  }
  for (const check of c.checks) {
    // A check that did not run is never shown as passed.
    const result =
      check.status === 'passed'
        ? 'PASS'
        : check.status === 'failed'
          ? 'FAIL'
          : `NOT EXECUTED — ${escapeMarkdown(check.reason?.trim() || 'no reason recorded')}`;
    lines.push(`- ${escapeMarkdown(check.name)}: ${result}`);
  }
  return lines.join('\n');
}
