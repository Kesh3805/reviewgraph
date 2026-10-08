import { explanationOf, type PublishableFinding } from '../../src/publisher/publish-source';
import { checkRunResult, runMarker } from '../../src/publisher/publisher.service';
import { renderSummary, type SummaryInput } from '../../src/publisher/render/summary';
import {
  classifyStale,
  previouslyReported,
  type PreviousFinding,
} from '../../src/publisher/stale-resolution.service';

const finding = (over: Partial<PublishableFinding> = {}): PublishableFinding => ({
  id: 'vf-1',
  candidateId: 'cf-1',
  shortId: 'RG-1-001',
  fingerprint: 'v1:new',
  reviewRunId: 'run-b',
  severity: 'high',
  title: 'Double charge',
  whatChanged: 'w',
  whyRisky: 'r',
  behaviorResult: 'b',
  correctiveDirection: 'c',
  reviewer: 'correctness',
  confidence: 0.9,
  category: 'correctness',
  affectedSymbols: ['billing::Invoice::total'],
  ...over,
});

const previous = (over: Partial<PreviousFinding> = {}): PreviousFinding => ({
  publishedId: 'pf-1',
  providerCommentId: '101',
  path: 'src/invoice.ts',
  line: 6,
  headSha: 'a'.repeat(40),
  fingerprint: 'v1:old',
  category: 'correctness',
  severity: 'high',
  title: 'Old finding',
  affectedSymbols: ['billing::Invoice::charge'],
  ...over,
});

const noLineage = new Map<string, string>();

describe('stale classification (GH-011)', () => {
  it('still_present_not_reposted: same fingerprint is carried over', () => {
    const plan = classifyStale(
      [previous({ fingerprint: 'v1:same' })],
      [finding({ fingerprint: 'v1:same' })],
      noLineage,
      () => new Set(['src/invoice.ts']),
    );
    expect([...plan.carriedOver.keys()]).toEqual(['v1:same']);
    expect(plan.fixed).toEqual([]);
    expect(plan.unknown).toEqual([]);
  });

  it('renamed_symbol_matches_via_lineage', () => {
    const prev = previous({ affectedSymbols: ['billing::Invoice::charge'] });
    const cur = finding({ affectedSymbols: ['billing::Invoice::chargeOnce'] });
    const changed = () => new Set(['src/invoice.ts']);
    expect(classifyStale([prev], [cur], noLineage, changed).fixed).toHaveLength(1);
    const lineage = new Map([['billing::Invoice::charge', 'billing::Invoice::chargeOnce']]);
    const plan = classifyStale([prev], [cur], lineage, changed);
    expect(plan.carriedOver.get(cur.fingerprint)).toEqual([prev]);
    // A different category never matches through symbols alone.
    expect(
      classifyStale([prev], [finding({ ...cur, category: 'security' })], lineage, changed).fixed,
    ).toHaveLength(1);
  });

  it('fixed_finding_thread_resolved: absent and its file changed', () => {
    const plan = classifyStale([previous()], [], noLineage, () => new Set(['src/invoice.ts']));
    expect(plan.fixed.map((p) => p.publishedId)).toEqual(['pf-1']);
  });

  it('unknown_left_open_listed_in_summary: absent but unchanged, or comparison unavailable', () => {
    const unchanged = classifyStale([previous()], [], noLineage, () => new Set(['other.ts']));
    expect(unchanged.unknown.map((p) => p.publishedId)).toEqual(['pf-1']);
    const unavailable = classifyStale([previous()], [], noLineage, () => null);
    expect(unavailable.unknown).toHaveLength(1);

    const summary: SummaryInput = {
      runId: 'run-b',
      headSha: 'b'.repeat(40),
      degraded: false,
      changed: { files: 1, behavioralSymbols: 0, apiContracts: 0 },
      findingsBySeverity: {},
      verified: 0,
      candidates: 0,
      suppressed: {},
      outsideDiff: [],
      coverage: {
        reviewersRun: ['correctness'],
        reviewersNotRun: [],
        unreviewedClusters: [],
        checks: [],
      },
      previouslyReported: previouslyReported(unchanged),
    };
    const text = renderSummary(summary);
    expect(text).toContain('**Previously reported**');
    expect(text).toContain('Old finding (`src/invoice.ts:6`)');
    expect(renderSummary({ ...summary, previouslyReported: [] })).not.toContain('Previously');
  });
});

describe('check run conclusion (GH-009, INV-012)', () => {
  it('check_run_neutral_with_findings', () => {
    expect(checkRunResult({ failed: false, degraded: false, findings: 2 })).toEqual({
      conclusion: 'neutral',
      title: '2 findings published',
    });
  });

  it('check_run_success_only_when_clean_and_complete', () => {
    expect(checkRunResult({ failed: false, degraded: false, findings: 0 }).conclusion).toBe(
      'success',
    );
    expect(checkRunResult({ failed: false, degraded: true, findings: 0 })).toEqual({
      conclusion: 'neutral',
      title: 'Review incomplete',
    });
  });

  it('check_run_never_success_on_failure', () => {
    for (const degraded of [true, false]) {
      for (const findings of [0, 3]) {
        expect(checkRunResult({ failed: true, degraded, findings })).toEqual({
          conclusion: 'neutral',
          title: 'Review could not be published',
        });
      }
    }
  });

  it('runMarker', () => {
    expect(runMarker('abc')).toBe('reviewgraph:run=abc');
  });
});

describe('explanation mapping', () => {
  it('reads the stored PRD §59 explanation', () => {
    expect(
      explanationOf(
        {
          explanation: {
            what_changed: 'w',
            why_risky: 'r',
            behavior_result: 'b',
            corrective_direction: 'c',
            evidence_path: ['A', 'B'],
            latent: true,
          },
          items: [{ claim: 'calls charge', path: 'a.ts', line: 3 }],
        },
        'desc',
      ),
    ).toEqual({
      whatChanged: 'w',
      whyRisky: 'r',
      behaviorResult: 'b',
      correctiveDirection: 'c',
      evidencePath: ['A', 'B'],
      evidenceItems: [{ claim: 'calls charge', path: 'a.ts', line: 3 }],
      latent: true,
    });
  });

  it('falls back to the description and leaves missing parts empty (relocated, not dropped)', () => {
    expect(explanationOf([{ claim: 'c1' }, { nope: 1 }], 'desc')).toEqual({
      whatChanged: 'desc',
      whyRisky: '',
      behaviorResult: '',
      correctiveDirection: '',
      evidenceItems: [{ claim: 'c1' }],
    });
  });
});
