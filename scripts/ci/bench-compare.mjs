#!/usr/bin/env node
// Compares benchmark results to committed baselines.
// perf: p95 latency regression > 20% or memory regression > 15% fails.
// quality: precision drop > 3 points or false-positive rate rise > 3 points fails.
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export const THRESHOLDS = { p95Pct: 20, memPct: 15, precisionPts: 3, fpPts: 3 };

export function comparePerf(baseline, current, t = THRESHOLDS) {
  const failures = [];
  for (const [name, base] of Object.entries(baseline)) {
    const cur = current[name];
    if (!cur) continue;
    if (base.p95_ms && cur.p95_ms > base.p95_ms * (1 + t.p95Pct / 100)) {
      failures.push(`${name}: p95 ${base.p95_ms}ms -> ${cur.p95_ms}ms exceeds +${t.p95Pct}%`);
    }
    if (base.peak_mem_mb && cur.peak_mem_mb > base.peak_mem_mb * (1 + t.memPct / 100)) {
      failures.push(
        `${name}: memory ${base.peak_mem_mb}MB -> ${cur.peak_mem_mb}MB exceeds +${t.memPct}%`,
      );
    }
  }
  return failures;
}

// precision and fp_rate are percentages (0-100).
export function compareQuality(baseline, current, t = THRESHOLDS) {
  const failures = [];
  if (baseline.precision - current.precision > t.precisionPts) {
    failures.push(
      `precision ${baseline.precision} -> ${current.precision} drops more than ${t.precisionPts} points`,
    );
  }
  if (current.fp_rate - baseline.fp_rate > t.fpPts) {
    failures.push(
      `fp_rate ${baseline.fp_rate} -> ${current.fp_rate} rises more than ${t.fpPts} points`,
    );
  }
  return failures;
}

export function loadBaseline(path) {
  if (!existsSync(path)) {
    throw new Error(
      `missing baseline ${path}: bootstrap it by running the nightly benchmark and committing the output through a reviewed PR`,
    );
  }
  return JSON.parse(readFileSync(path, 'utf8'));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [kind, baselinePath, currentPath] = process.argv.slice(2);
  try {
    const base = loadBaseline(baselinePath);
    const cur = JSON.parse(readFileSync(currentPath, 'utf8'));
    const failures = kind === 'quality' ? compareQuality(base, cur) : comparePerf(base, cur);
    for (const f of failures) console.error(`REGRESSION ${f}`);
    console.log(failures.length ? `${failures.length} regression(s)` : 'no regressions');
    process.exit(failures.length ? 1 : 0);
  } catch (e) {
    console.error(e.message);
    process.exit(2);
  }
}
