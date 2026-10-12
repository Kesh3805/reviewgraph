#!/usr/bin/env node
// Validates dashboards, alerts and runbooks. Usage: node infra/openobserve/validate.mjs
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(ROOT, '..', '..');
export const REQUIRED_RUNBOOK_HEADINGS = [
  'Symptom',
  'Impact',
  'Likely causes',
  'Diagnosis',
  'Mitigation',
  'Escalation',
  'Related alerts',
];

export function catalogNames(catalogPath = join(REPO, 'docs/operations/metrics-catalog.md')) {
  const text = readFileSync(catalogPath, 'utf8');
  return new Set([...text.matchAll(/`([a-z][a-z0-9_]*)`/g)].map((m) => m[1]));
}

const readJsonDir = (dir) =>
  existsSync(dir)
    ? readdirSync(dir)
        .filter((f) => f.endsWith('.json'))
        .sort()
        .map((f) => ({ file: f, path: join(dir, f) }))
    : [];

export function validate({ baseDir = ROOT, repoDir = REPO, names = catalogNames(join(repoDir, 'docs/operations/metrics-catalog.md')) } = {}) {
  const errors = [];
  const ids = new Set();
  for (const { file, path } of readJsonDir(join(baseDir, 'dashboards'))) {
    let d;
    try {
      d = JSON.parse(readFileSync(path, 'utf8'));
    } catch (e) {
      errors.push(`${file}: invalid JSON`);
      continue;
    }
    if (!d.dashboardId?.startsWith('rg-')) errors.push(`${file}: dashboardId must start with rg-`);
    if (ids.has(d.dashboardId)) errors.push(`${file}: duplicate dashboardId ${d.dashboardId}`);
    ids.add(d.dashboardId);
    for (const tab of d.tabs ?? []) {
      for (const panel of tab.panels ?? []) {
        for (const q of panel.queries ?? []) {
          const s = q.fields?.stream;
          if (q.fields?.stream_type === 'metrics' && !names.has(s)) {
            errors.push(`${file}: unknown metric ${s}`);
          }
        }
      }
    }
  }

  const referenced = new Set();
  const alertNames = new Set();
  for (const { file, path } of readJsonDir(join(baseDir, 'alerts'))) {
    let a;
    try {
      a = JSON.parse(readFileSync(path, 'utf8'));
    } catch (e) {
      errors.push(`${file}: invalid JSON`);
      continue;
    }
    if (!a.name?.startsWith('rg-')) errors.push(`${file}: name must start with rg-`);
    if (alertNames.has(a.name)) errors.push(`${file}: duplicate alert ${a.name}`);
    alertNames.add(a.name);
    if (!names.has(a.metric)) errors.push(`${file}: unknown metric ${a.metric}`);
    if (typeof a.threshold?.value !== 'number' || !['>', '>=', '<', '<=', '='].includes(a.threshold?.operator)) {
      errors.push(`${file}: threshold must be {operator, value:number}`);
    }
    if (!['page', 'ticket'].includes(a.severity)) errors.push(`${file}: severity must be page|ticket`);
    const rb = a.annotations?.runbook_url;
    if (!rb || !existsSync(join(repoDir, rb))) errors.push(`${file}: runbook missing (${rb})`);
    else referenced.add(rb.replaceAll('\\', '/'));
  }
  const rbDir = join(repoDir, 'docs/operations/runbooks');
  if (existsSync(rbDir) && alertNames.size > 0) {
    for (const f of readdirSync(rbDir).filter((x) => x.endsWith('.md'))) {
      const rel = `docs/operations/runbooks/${f}`;
      if (!referenced.has(rel)) errors.push(`${rel}: not referenced by any alert`);
      const text = readFileSync(join(rbDir, f), 'utf8');
      for (const h of REQUIRED_RUNBOOK_HEADINGS) {
        if (!new RegExp(`^##\\s+${h}\\s*$`, 'm').test(text)) errors.push(`${rel}: missing heading "${h}"`);
      }
    }
  }
  for (const spec of ['destinations', 'templates']) {
    for (const { file, path } of readJsonDir(join(baseDir, spec))) {
      const text = readFileSync(path, 'utf8');
      if (/https?:\/\/(?!localhost|127\.0\.0\.1)[^\s"]+/.test(text) || /(token|secret|password)"\s*:\s*"[^"$]/i.test(text)) {
        errors.push(`${spec}/${file}: contains a remote URL or literal secret; use env placeholders`);
      }
    }
  }
  return errors;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const errors = validate();
  if (errors.length) {
    console.error(errors.join('\n'));
    process.exit(1);
  }
  console.log('openobserve assets valid');
}
