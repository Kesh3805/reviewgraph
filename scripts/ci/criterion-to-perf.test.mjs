import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { collect, p95Millis } from './criterion-to-perf.mjs';

test('p95Millis uses per-iteration time in milliseconds', () => {
  // 20 samples of 1ms/iter except the last one at 10ms/iter; p95 is the 19th value.
  const iters = Array(20).fill(10);
  const times = iters.map((n, i) => (i === 19 ? 100_000_000 : 10_000_000));
  assert.equal(p95Millis({ iters, times }), 1);
});

test('collect walks nested benchmark groups and skips the report directory', () => {
  const root = mkdtempSync(join(tmpdir(), 'crit-'));
  const dir = join(root, 'group', 'bench_a', 'new');
  mkdirSync(dir, { recursive: true });
  mkdirSync(join(root, 'report'), { recursive: true });
  writeFileSync(join(dir, 'sample.json'), JSON.stringify({ iters: [1, 1], times: [2e6, 4e6] }));
  writeFileSync(join(dir, 'estimates.json'), JSON.stringify({ median: { point_estimate: 3e6 } }));
  const r = collect(root);
  assert.deepEqual(Object.keys(r), ['group/bench_a']);
  assert.equal(r['group/bench_a'].median_ms, 3);
  assert.equal(r['group/bench_a'].p95_ms, 4);
});
