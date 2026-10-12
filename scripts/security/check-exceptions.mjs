#!/usr/bin/env node
// Validates security/exceptions.toml and the threat model references.
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const DAY = 86_400_000;

export function parseExceptions(text) {
  const entries = [];
  let cur = null;
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    if (line === '[[exception]]') {
      cur = {};
      entries.push(cur);
    } else if (cur) {
      const m = line.match(/^(\w+)\s*=\s*"(.*)"$/);
      if (m) cur[m[1]] = m[2];
    }
  }
  return entries;
}

export function checkExceptions(text, now = new Date()) {
  const errors = [];
  for (const [i, e] of parseExceptions(text).entries()) {
    const where = `exception[${i}] ${e.id ?? '?'}`;
    for (const f of ['id', 'reason', 'owner', 'added', 'expires']) {
      if (!e[f]) errors.push(`${where}: missing ${f}`);
    }
    const added = Date.parse(e.added);
    const expires = Date.parse(e.expires);
    if (Number.isNaN(expires) || Number.isNaN(added)) {
      errors.push(`${where}: malformed date`);
      continue;
    }
    if (expires < now.getTime()) errors.push(`${where}: expired ${e.expires}`);
    if (expires - added > 90 * DAY) errors.push(`${where}: expiry exceeds 90 days`);
  }
  return errors;
}

export function checkThreatModel(text, planText) {
  const errors = [];
  const known = new Set([...planText.matchAll(/Task ID:\*\* ([A-Z]+-\d+)/g)].map((m) => m[1]));
  const ids = [...text.matchAll(/^\| (T-\d+) \|/gm)].map((m) => m[1]);
  if (new Set(ids).size !== ids.length) errors.push('duplicate threat ids');
  for (const row of text.split('\n').filter((l) => /^\| T-\d+ /.test(l))) {
    for (const t of row.match(/\b[A-Z]{2,5}-\d{3}\b/g) ?? []) {
      if (!known.has(t)) errors.push(`unknown task id ${t}`);
    }
  }
  return errors;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const planDir = join(REPO, 'docs/planning/tasks');
  const plan = readdirSync(planDir)
    .filter((f) => f.endsWith('.md'))
    .map((f) => readFileSync(join(planDir, f), 'utf8'))
    .join('\n');
  const errors = [
    ...checkExceptions(readFileSync(join(REPO, 'security/exceptions.toml'), 'utf8')),
    ...checkThreatModel(readFileSync(join(REPO, 'docs/security/threat-model.md'), 'utf8'), plan),
  ];
  if (errors.length) {
    console.error(errors.join('\n'));
    process.exit(1);
  }
  console.log('security exceptions and threat model valid');
}
