#!/usr/bin/env node
// Prints a markdown job summary (counts, slowest tests, coverage) for JUnit XML files.
//   node scripts/ci/junit-summary.mjs <junit-dir> [coverage-dir] >> "$GITHUB_STEP_SUMMARY"
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const attr = (tag, name) => {
  const m = new RegExp(`\\b${name}="([^"]*)"`).exec(tag);
  return m ? m[1] : undefined;
};

/** Parses one JUnit document into totals and per-testcase timings. */
export function parseJunit(xml) {
  const cases = [];
  for (const m of xml.matchAll(/<testcase\b[^>]*>/g)) {
    cases.push({
      name: attr(m[0], 'name') ?? '?',
      suite: attr(m[0], 'classname') ?? '',
      time: Number(attr(m[0], 'time') ?? 0),
    });
  }
  const failures = (xml.match(/<failure\b/g) ?? []).length + (xml.match(/<error\b/g) ?? []).length;
  const skipped = (xml.match(/<skipped\b/g) ?? []).length;
  return { tests: cases.length, failures, skipped, cases };
}

export function summarize(dir, coverageDir) {
  const files = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith('.xml')) : [];
  const lines = [
    '### Unit test results',
    '',
    '| Suite file | Tests | Failed | Skipped |',
    '|---|---|---|---|',
  ];
  const all = [];
  for (const f of files) {
    const r = parseJunit(readFileSync(join(dir, f), 'utf8'));
    lines.push(`| ${f} | ${r.tests} | ${r.failures} | ${r.skipped} |`);
    all.push(...r.cases.map((c) => ({ ...c, file: f })));
  }
  if (files.length === 0) lines.push('| (no JUnit files) | 0 | 0 | 0 |');
  const slow = [...all].sort((a, b) => b.time - a.time).slice(0, 10);
  if (slow.length) {
    lines.push('', 'Slowest 10 tests:', '', '| Seconds | Test |', '|---|---|');
    for (const c of slow) lines.push(`| ${c.time.toFixed(2)} | ${c.file}: ${c.name} |`);
  }
  const cov = coverageDir && join(coverageDir, 'api', 'coverage-summary.json');
  if (cov && existsSync(cov)) {
    const t = JSON.parse(readFileSync(cov, 'utf8')).total;
    lines.push(
      '',
      `Coverage (advisory, api): lines ${t.lines.pct}%, branches ${t.branches.pct}%, functions ${t.functions.pct}%`,
    );
  }
  return lines.join('\n') + '\n';
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [dir = 'test-results', coverageDir] = process.argv.slice(2);
  process.stdout.write(summarize(dir, coverageDir));
}
