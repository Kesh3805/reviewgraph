import {
  NO_MERGE_FOOTER,
  renderSummary,
  type SummaryInput,
} from '../../src/publisher/render/summary';

const base = (over: Partial<SummaryInput> = {}): SummaryInput => ({
  runId: 'run-77',
  headSha: 'ec26c3e57ca3',
  degraded: false,
  changed: { files: 17, behavioralSymbols: 8, apiContracts: 2 },
  riskAreas: ['authentication', 'database writes'],
  findingsBySeverity: { high: 1, medium: 1 },
  verified: 2,
  candidates: 6,
  suppressed: { low_confidence: 4 },
  outsideDiff: [],
  coverage: {
    reviewersRun: ['correctness', 'security'],
    reviewersNotRun: [],
    unreviewedClusters: [],
    checks: [],
  },
  ...over,
});

describe('renderSummary', () => {
  it('summary_prd61_example_snapshot', () => {
    expect(renderSummary(base())).toBe(
      [
        '## ReviewGraph',
        '**Changed**\n- 17 files\n- 8 behavioral symbols\n- 2 API contracts',
        '**Risk areas**\n- authentication\n- database writes',
        '**Findings**\n- 1 high\n- 1 medium',
        '**Verified**\n2 / 6 candidate findings',
        '**Suppressed**\n- 4 low-confidence candidates',
        '**Coverage**\n- Reviewers run: correctness, security',
        NO_MERGE_FOOTER,
        '<!-- reviewgraph:run=run-77 head=ec26c3e57ca3 -->',
      ].join('\n\n'),
    );
  });

  it('summary_lists_outside_diff_findings', () => {
    const body = renderSummary(
      base({
        outsideDiff: [
          {
            severity: 'high',
            title: 'Missing check',
            path: 'src/a.ts',
            line: 42,
            reason: 'outside_diff',
          },
          { severity: 'low', title: 'Cap overflow', reason: 'inline_cap' },
        ],
      }),
    );
    expect(body).toContain('**Findings outside the diff**');
    expect(body).toContain('- \u{1F534} **High** — Missing check (`src/a.ts:42`)');
    expect(body).toContain('- \u{1F7E1} **Low** — Cap overflow');
  });

  it('summary_not_executed_never_pass', () => {
    const body = renderSummary(
      base({
        coverage: {
          reviewersRun: [],
          reviewersNotRun: [],
          unreviewedClusters: [],
          checks: [
            { name: 'tsc', status: 'not_executed', reason: 'toolchain missing' },
            { name: 'eslint', status: 'not_executed' },
            { name: 'jest', status: 'passed' },
            { name: 'clippy', status: 'failed' },
          ],
        },
      }),
    );
    expect(body).toContain('- tsc: NOT EXECUTED — toolchain missing');
    expect(body).toContain('- eslint: NOT EXECUTED — no reason recorded');
    expect(body).toContain('- jest: PASS');
    expect(body).toContain('- clippy: FAIL');
    expect(body).not.toMatch(/tsc: PASS|eslint: PASS/);
  });

  it('summary_degraded_header', () => {
    expect(renderSummary(base({ degraded: true }))).toContain(
      '> Review completed with reduced coverage.',
    );
    expect(renderSummary(base())).not.toContain('reduced coverage');
  });

  it('summary_zero_findings_shows_coverage', () => {
    const body = renderSummary(
      base({
        findingsBySeverity: {},
        verified: 0,
        candidates: 3,
        coverage: {
          reviewersRun: ['correctness'],
          reviewersNotRun: [{ reviewer: 'security', reason: 'model timeout' }],
          unreviewedClusters: ['billing/'],
          checks: [],
        },
      }),
    );
    expect(body).toContain('No verified findings.');
    expect(body).toContain('- Not run: security — model timeout');
    expect(body).toContain('- Unreviewed clusters: billing/');
    expect(body).toContain('0 / 3 candidate findings');
  });

  it('summary_no_merge_footer and hidden marker', () => {
    const body = renderSummary(base());
    expect(body).toContain('No merge performed. ReviewGraph never approves or merges.');
    expect(body.trimEnd().endsWith('<!-- reviewgraph:run=run-77 head=ec26c3e57ca3 -->')).toBe(true);
  });

  it('summary_counts_match_db: rendered numbers are exactly the supplied counts', () => {
    const body = renderSummary(
      base({
        findingsBySeverity: { critical: 2, high: 3, low: 5 },
        verified: 10,
        candidates: 40,
        suppressed: {
          low_confidence: 7,
          duplicate: 3,
          pre_existing: 2,
          not_actionable: 1,
          policy: 4,
        },
      }),
    );
    expect(body).toContain('- 2 critical\n- 3 high\n- 5 low');
    expect(body).toContain('10 / 40 candidate findings');
    expect(body).toContain(
      '- 7 low-confidence candidates\n- 3 duplicates\n- 2 pre-existing issues\n- 1 not actionable\n- 4 suppressed by policy',
    );
  });

  it('caps risk areas at five, handles unavailable assessment and policy notice', () => {
    const many = renderSummary(base({ riskAreas: ['a', 'b', 'c', 'd', 'e', 'f', 'g'] }));
    expect(many).toContain('- e');
    expect(many).not.toContain('- f');
    const missing = renderSummary(base({ riskAreas: undefined }));
    expect(missing).toContain('Risk areas: unavailable');
    expect(renderSummary(base({ policyChange: 'Rules changed in this PR' }))).toContain(
      '> **Policy change:** Rules changed in this PR',
    );
  });

  it('escapes untrusted text and cannot close the marker', () => {
    const body = renderSummary(
      base({
        runId: 'r --> x',
        riskAreas: ['<img src=x> @octocat'],
        outsideDiff: [{ severity: 'info', title: '--> <b>', reason: 'outside_diff' }],
      }),
    );
    expect(body).not.toContain('<img');
    expect(body).not.toContain('@octocat');
    expect(body.match(/<!--/g)).toHaveLength(1);
    expect(body.match(/-->/g)).toHaveLength(1);
  });
});
