import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { counterTotal, resetCounterTotals } from '../../src/common/metrics';
import {
  addMetric,
  boundLabel,
  DURATION_BUCKETS_SECONDS,
  FORBIDDEN_LABELS,
  LABEL_ALLOW_LIST,
  TS_METRICS,
  type MetricKind,
  type TsMetricName,
} from '../../src/telemetry/metrics';

const REPO = join(__dirname, '../../../..');
const doc = readFileSync(join(REPO, 'docs/operations/metrics-catalog.md'), 'utf8');
const rust = readFileSync(join(REPO, 'engine/crates/telemetry/src/metrics/instruments.rs'), 'utf8');

interface Row {
  kind: MetricKind;
  unit: string;
  labels: string[];
}

function catalogRows(): Map<string, Row> {
  const rows = new Map<string, Row>();
  for (const line of doc.split('\n')) {
    const m = /^\| `([a-z_]+)` \| (\w+) \| (\S+) \| ([^|]*)\|/.exec(line);
    if (!m) continue;
    const labels = (m[4] ?? '')
      .split(',')
      .map((l) => l.trim())
      .filter(Boolean);
    rows.set(m[1]!, { kind: m[2] as MetricKind, unit: m[3]!, labels });
  }
  return rows;
}

function sharedWithTypeScript(): string[] {
  const block = /SHARED_WITH_TYPESCRIPT: &\[&str\] = &\[([\s\S]*?)\];/.exec(rust)?.[1] ?? '';
  return [...block.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]!);
}

describe('TS metric registry', () => {
  const rows = catalogRows();

  it('ts_registry_exposes_same_names_as_rust_for_shared_metrics', () => {
    const shared = sharedWithTypeScript();
    expect(shared.length).toBeGreaterThan(5);
    for (const name of shared) {
      expect(Object.keys(TS_METRICS)).toContain(name);
    }
  });

  it('agrees with the generated catalog on kind, unit and labels', () => {
    for (const [name, spec] of Object.entries(TS_METRICS)) {
      const row = rows.get(name);
      expect(row).toBeDefined();
      expect({ name, kind: row?.kind, unit: row?.unit, labels: row?.labels }).toEqual({
        name,
        kind: spec.kind,
        unit: spec.unit,
        labels: [...spec.labels],
      });
    }
  });

  it('uses only allow-listed labels and never an identifier', () => {
    for (const spec of Object.values(TS_METRICS)) {
      for (const label of spec.labels) {
        expect(LABEL_ALLOW_LIST as readonly string[]).toContain(label);
        expect(FORBIDDEN_LABELS as readonly string[]).not.toContain(label);
      }
    }
  });

  it('shares the duration buckets with the catalog', () => {
    const m = /buckets \(seconds\): (.*)\.$/m.exec(doc);
    expect(m?.[1]?.split(', ').map(Number)).toEqual([...DURATION_BUCKETS_SECONDS]);
  });

  it('maps unknown label values to other and drops unknown label names', () => {
    expect(boundLabel('queue', 'pr-review')).toBe('pr-review');
    expect(boundLabel('queue', 'org-123')).toBe('other');
    expect(boundLabel('model', 'src/a.ts')).toBe('other');
    resetCounterTotals();
    addMetric('trace_context_missing_total', { queue: 'nope', organization_id: 'o1' });
    expect(counterTotal('trace_context_missing_total', { queue: 'other' })).toBe(1);
  });

  it('rejects the wrong instrument kind', () => {
    expect(() => addMetric('queue_depth' as TsMetricName)).toThrow(/gauge/);
  });
});
