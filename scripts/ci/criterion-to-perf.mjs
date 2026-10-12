#!/usr/bin/env node
// Converts criterion output (engine/target/criterion) into the perf baseline format used by
// bench-compare.mjs: { "<group>/<bench>": { p95_ms, median_ms, samples } }.
//   node scripts/ci/criterion-to-perf.mjs engine/target/criterion > perf-current.json
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

/** p95 of per-iteration times (nanoseconds) from a criterion sample.json, in milliseconds. */
export function p95Millis(sample) {
  const per = sample.times.map((t, i) => t / sample.iters[i]).sort((a, b) => a - b);
  if (per.length === 0) return 0;
  const idx = Math.min(per.length - 1, Math.ceil(per.length * 0.95) - 1);
  return per[idx] / 1e6;
}

const round = (n) => Math.round(n * 1e6) / 1e6;

export function collect(root) {
  const out = {};
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const p = join(dir, name);
      if (!statSync(p).isDirectory()) continue;
      const sample = join(p, 'new', 'sample.json');
      const est = join(p, 'new', 'estimates.json');
      if (existsSync(sample) && existsSync(est)) {
        const key = relative(root, p).split('\\').join('/');
        const s = JSON.parse(readFileSync(sample, 'utf8'));
        const e = JSON.parse(readFileSync(est, 'utf8'));
        out[key] = {
          p95_ms: round(p95Millis(s)),
          median_ms: round(e.median.point_estimate / 1e6),
          samples: s.times.length,
        };
      } else if (name !== 'report') {
        walk(p);
      }
    }
  };
  if (existsSync(root)) walk(root);
  return Object.fromEntries(Object.entries(out).sort(([a], [b]) => a.localeCompare(b)));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = process.argv[2] ?? 'engine/target/criterion';
  const result = collect(root);
  if (Object.keys(result).length === 0) {
    console.error(`no criterion results under ${root}`);
    process.exit(1);
  }
  process.stdout.write(JSON.stringify(result, null, 2) + '\n');
}
