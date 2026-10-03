import { DiffIndex, parsePatch } from '../../src/publisher/render/diff-index';
import { anchorFinding } from '../../src/publisher/render/anchor';
import {
  LATENT_NOTE,
  RenderError,
  renderInlineComment,
} from '../../src/publisher/render/inline-comment';
import { planPublication } from '../../src/publisher/render/plan';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import { PATCH, authBypass } from './render-fixtures';

const diff = DiffIndex.build([{ path: 'src/auth.ts', patch: PATCH }]);
const at = (
  startLine: number,
  endLine: number,
  side: 'head' | 'base' = 'head',
  path = 'src/auth.ts',
) => authBypass({ location: { path, startLine, endLine, side } });

describe('renderInlineComment', () => {
  it('renders_prd58_example', () => {
    expect(renderInlineComment(authBypass())).toBe(
      [
        '\u{1F534} **High — Authorization check bypassed**',
        'authorize() now accepts user.role === "admin" directly and no longer calls PermissionService.check().  AdminService.updateUser() reaches this path from the user-management endpoint, so resource-level permissions are no longer evaluated for that operation.',
        'Any admin can update any user, including users outside their tenant.',
        '**Evidence**\n```text\nAuthService.authorize()\n  → AdminService.updateUser()\n  → UserController.update()\n```',
        'Preserve the resource permission check before permitting the mutation.',
        '<sub>RG-12-001 · security · confidence 0.91</sub>\n<!-- reviewgraph:finding=fp-abc123 run=run-77 -->',
      ].join('\n\n'),
    );
  });

  it('answers_all_five_questions_fields_required', () => {
    for (const field of [
      'whatChanged',
      'whyRisky',
      'behaviorResult',
      'correctiveDirection',
      'title',
    ] as const) {
      expect(() => renderInlineComment(authBypass({ [field]: '  ' }))).toThrow(RenderError);
    }
  });

  it('uses evidence items when there is no path, and omits the block with neither', () => {
    const cited = renderInlineComment(
      authBypass({
        evidencePath: undefined,
        evidenceItems: [{ claim: 'Caller skips the check', path: 'src/admin.ts', line: 9 }],
      }),
    );
    expect(cited).toContain('**Evidence**\n- Caller skips the check (`src/admin.ts:9`)');
    expect(renderInlineComment(authBypass({ evidencePath: undefined }))).not.toContain('Evidence');
  });

  it('latent_note_present', () => {
    expect(renderInlineComment(authBypass({ latent: true }))).toContain(LATENT_NOTE);
    expect(renderInlineComment(authBypass())).not.toContain('Latent');
  });

  it('marker_cannot_be_closed_by_text', () => {
    const body = renderInlineComment(
      authBypass({
        title: 'x --> <script>alert(1)</script> @octocat',
        whyRisky: 'y --><!-- reviewgraph:finding=evil -->',
        fingerprint: 'fp --> <b>',
        reviewRunId: 'run\n-->',
      }),
    );
    expect(body).not.toContain('<script>');
    expect(body).not.toContain('@octocat');
    // Exactly one comment opener and one closer: the real marker.
    expect(body.match(/<!--/g)).toHaveLength(1);
    expect(body.match(/-->/g)).toHaveLength(1);
    expect(body.trimEnd().endsWith('-->')).toBe(true);
  });

  it('escapes markdown punctuation in finding text', () => {
    const body = renderInlineComment(authBypass({ title: 'a*b_c`d\\e [x] #1' }));
    expect(body).toContain('**High — a\\*b\\_c\\`d\\\\e \\[x\\] \\#1**');
  });

  it('rejects hedging phrases in development builds', () => {
    expect(() =>
      renderInlineComment(authBypass({ whyRisky: 'This potentially might break auth.' })),
    ).toThrow(/hedging/);
  });

  it('is deterministic', () => {
    expect(renderInlineComment(authBypass())).toBe(renderInlineComment(authBypass()));
  });
});

describe('diff index and anchoring', () => {
  it('parses hunks with old and new numbering', () => {
    const hunks = parsePatch(PATCH);
    expect(hunks).toHaveLength(2);
    expect(hunks[0]!.lines.filter((l) => l.kind === 'add').map((l) => l.newLine)).toEqual([13, 14]);
    expect(hunks[1]!.lines.filter((l) => l.kind === 'del').map((l) => l.oldLine)).toEqual([40, 41]);
  });

  it('anchors an added line on the RIGHT side', () => {
    expect(anchorFinding(at(13, 13), diff)).toEqual({
      kind: 'inline',
      path: 'src/auth.ts',
      line: 13,
      side: 'RIGHT',
    });
  });

  it('multiline_anchor_same_hunk', () => {
    expect(anchorFinding(at(12, 15), diff)).toEqual({
      kind: 'inline',
      path: 'src/auth.ts',
      line: 15,
      side: 'RIGHT',
      startLine: 12,
      startSide: 'RIGHT',
    });
  });

  it('multiline_across_hunks_falls_back_single_line', () => {
    // 11..41 spans both hunks: the first changed visible line is used.
    expect(anchorFinding(at(11, 41), diff)).toEqual({
      kind: 'inline',
      path: 'src/auth.ts',
      line: 13,
      side: 'RIGHT',
    });
    // A range only partly inside a hunk (ends in the gap) also falls back.
    expect(anchorFinding(at(15, 25), diff)).toEqual({
      kind: 'inline',
      path: 'src/auth.ts',
      line: 15,
      side: 'RIGHT',
    });
  });

  it('deletion_anchors_left_side', () => {
    expect(anchorFinding(at(41, 41, 'base'), diff)).toEqual({
      kind: 'inline',
      path: 'src/auth.ts',
      line: 41,
      side: 'LEFT',
    });
    expect(anchorFinding(at(40, 41, 'base'), diff)).toMatchObject({
      side: 'LEFT',
      line: 41,
      startLine: 40,
      startSide: 'LEFT',
    });
  });

  it('outside_diff_goes_to_summary', () => {
    const outside = { kind: 'summary', reason: 'outside_diff' };
    expect(anchorFinding(at(200, 200), diff)).toEqual(outside);
    expect(anchorFinding(at(1, 3), diff)).toEqual(outside);
    expect(anchorFinding(at(13, 13, 'head', 'src/other.ts'), diff)).toEqual(outside);
    expect(anchorFinding(authBypass({ location: undefined }), diff)).toEqual(outside);
    // A file without a patch (binary or oversized) has no commentable lines.
    expect(anchorFinding(at(41, 41, 'base'), DiffIndex.build([{ path: 'src/auth.ts' }]))).toEqual(
      outside,
    );
  });
});

describe('planPublication', () => {
  beforeEach(() => resetCounterTotals());

  const added = { path: 'src/auth.ts', startLine: 13, endLine: 13, side: 'head' as const };

  it('only_inline_findings_become_anchored_comments', () => {
    const plan = planPublication(
      [
        authBypass({ id: 'a', location: added }),
        authBypass({ id: 'b', location: { ...added, startLine: 300, endLine: 300 } }),
        authBypass({ id: 'c', location: undefined }),
      ],
      diff,
    );
    expect(plan.inline.map((p) => p.finding.id)).toEqual(['a']);
    expect(plan.inline[0]!.anchor).toMatchObject({ side: 'RIGHT', line: 13 });
    expect(plan.relocated.map((r) => [r.finding.id, r.reason])).toEqual([
      ['b', 'outside_diff'],
      ['c', 'outside_diff'],
    ]);
    expect(counterTotal('findings_relocated_to_summary_total', { reason: 'outside_diff' })).toBe(2);
  });

  it('inline_comments_respect_the_cap and overflow is relocated, not dropped', () => {
    const findings = Array.from({ length: 40 }, (_, i) =>
      authBypass({ id: `f${i}`, location: added }),
    );
    const plan = planPublication(findings, diff, { inlineCap: 25 });
    expect(plan.inline).toHaveLength(25);
    expect(plan.relocated).toHaveLength(15);
    expect(plan.relocated.every((r) => r.reason === 'inline_cap')).toBe(true);
    expect(plan.inline.length + plan.relocated.length).toBe(40);
    // Priority order preserved: the first 25 stay inline.
    expect(plan.inline[0]!.finding.id).toBe('f0');
    expect(planPublication(findings, diff).inline).toHaveLength(25);
  });

  it('a finding with missing explanation fields is relocated with render_error', () => {
    const plan = planPublication([authBypass({ id: 'x', whyRisky: '' })], diff);
    expect(plan.inline).toHaveLength(0);
    expect(plan.relocated).toEqual([expect.objectContaining({ reason: 'render_error' })]);
    expect(counterTotal('findings_relocated_to_summary_total', { reason: 'render_error' })).toBe(1);
  });
});
