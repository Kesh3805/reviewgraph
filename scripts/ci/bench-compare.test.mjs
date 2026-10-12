import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { comparePerf, compareQuality, loadBaseline } from './bench-compare.mjs';

const base = { parse: { p95_ms: 100, peak_mem_mb: 200 } };

test('compare_fails_on_p95_regression_over_20_percent', () => {
  assert.equal(comparePerf(base, { parse: { p95_ms: 121, peak_mem_mb: 200 } }).length, 1);
});

test('compare_passes_within_noise_tolerance', () => {
  assert.equal(comparePerf(base, { parse: { p95_ms: 115, peak_mem_mb: 220 } }).length, 0);
});

test('compare_fails_on_precision_drop_over_3_points', () => {
  assert.equal(
    compareQuality({ precision: 80, fp_rate: 10 }, { precision: 76, fp_rate: 10 }).length,
    1,
  );
});

test('compare_fails_on_fp_rate_rise_over_3_points', () => {
  assert.equal(
    compareQuality({ precision: 80, fp_rate: 10 }, { precision: 80, fp_rate: 14 }).length,
    1,
  );
});

test('smoke_set_has_exactly_five_cases_that_exist', () => {
  const ids = readFileSync(
    new URL('../../benchmarks/quality/smoke-set.txt', import.meta.url),
    'utf8',
  )
    .split('\n')
    .filter((l) => l.trim() && !l.startsWith('#'));
  assert.equal(ids.length, 5);
  assert.equal(new Set(ids).size, 5);
});

test('missing_baseline_gives_bootstrap_message', () => {
  assert.throws(() => loadBaseline('/nonexistent/perf.json'), /bootstrap/);
});
