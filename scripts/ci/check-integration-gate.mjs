#!/usr/bin/env node
// CI-004 guards over Rust test files:
//  - integration_feature_gate_guard: a test target that reads DATABASE_URL must be gated by the
//    `integration` feature (cfg attribute or a skip-when-unset guard) so plain `cargo test` never
//    needs a database.
//  - quarantine_count_does_not_increase: `#[ignore = "quarantine:ISSUE-n"]` tests are counted and
//    compared with the budget in scripts/ci/quarantine-budget.txt.
import { readdirSync, readFileSync, statSync, existsSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const DB_ENV = /env::var(?:_os)?\(\s*"(?:TEST_)?DATABASE_URL"/;
const GATE =
  /feature\s*=\s*"integration"|let Some\(|let Ok\(|\.ok\(\)\s*else|is_err\(\)|is_none\(\)|return;/;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === 'target' || name === 'node_modules') continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (p.endsWith('.rs')) out.push(p);
  }
  return out;
}

// `tests/common/` holds helpers shared by gated test targets; the gate lives in the callers.
const isTestTarget = (rel) => {
  const parts = rel.split(sep);
  return parts.includes('tests') && !parts.includes('common');
};

/** Returns the test files that touch the database env var without any gate or skip guard. */
export function ungatedFiles(root, files = walk(root)) {
  const bad = [];
  for (const f of files) {
    const rel = relative(root, f);
    if (!isTestTarget(rel)) continue;
    const src = readFileSync(f, 'utf8');
    if (DB_ENV.test(src) && !GATE.test(src)) bad.push(rel);
  }
  return bad;
}

export function quarantined(root, files = walk(root)) {
  const hits = [];
  for (const f of files) {
    const src = readFileSync(f, 'utf8');
    for (const m of src.matchAll(/#\[ignore\s*=\s*"quarantine:([^"]*)"\]/g)) {
      hits.push({ file: relative(root, f), issue: m[1] });
    }
  }
  return hits;
}

export function budgetExceeded(count, budgetText) {
  const budget = Number.parseInt(budgetText.trim(), 10);
  return Number.isFinite(budget) && count > budget;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const repo = join(fileURLToPath(import.meta.url), '..', '..', '..');
  const engine = join(repo, 'engine');
  let failed = false;
  const bad = ungatedFiles(engine);
  for (const f of bad)
    console.error(`UNGATED ${f}: reads DATABASE_URL without the integration gate`);
  if (bad.length) failed = true;
  const q = quarantined(engine);
  const budgetFile = join(repo, 'scripts', 'ci', 'quarantine-budget.txt');
  const budget = existsSync(budgetFile) ? readFileSync(budgetFile, 'utf8') : '0';
  for (const h of q) console.log(`quarantined: ${h.file} (${h.issue})`);
  if (budgetExceeded(q.length, budget)) {
    console.error(`quarantine count ${q.length} exceeds the budget ${budget.trim()}`);
    failed = true;
  }
  console.log(`integration gate guard: ${bad.length} ungated, ${q.length} quarantined`);
  process.exit(failed ? 1 : 0);
}
