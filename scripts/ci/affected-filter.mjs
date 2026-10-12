#!/usr/bin/env node
// Maps changed files to workspace packages plus dependents and prints a pnpm filter string.
// Output: "" (nothing affected), "ALL" (run everything) or "--filter a --filter b ...".
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ALL_TRIGGERS = [
  /^pnpm-lock\.yaml$/,
  /^pnpm-workspace\.yaml$/,
  /^package\.json$/,
  /^packages\/config\//,
  /^\.github\/workflows\/ts\.yml$/,
  /^scripts\/ci\/affected-filter/,
];

export function loadPackages(root) {
  const pkgs = [];
  for (const group of ['apps', 'packages']) {
    const dir = join(root, group);
    if (!existsSync(dir)) continue;
    for (const name of readdirSync(dir)) {
      const file = join(dir, name, 'package.json');
      if (!existsSync(file)) continue;
      const json = JSON.parse(readFileSync(file, 'utf8'));
      const deps = Object.keys({
        ...json.dependencies,
        ...json.devDependencies,
        ...json.peerDependencies,
      });
      pkgs.push({ name: json.name, dir: `${group}/${name}`, deps });
    }
  }
  return pkgs;
}

export function affected(changedFiles, packages) {
  const files = changedFiles.map((f) => f.replace(/\\/g, '/')).filter(Boolean);
  if (files.some((f) => ALL_TRIGGERS.some((re) => re.test(f)))) return 'ALL';
  const hit = new Set();
  for (const p of packages) {
    if (files.some((f) => f.startsWith(`${p.dir}/`))) hit.add(p.name);
  }
  let grew = true;
  while (grew) {
    grew = false;
    for (const p of packages) {
      if (!hit.has(p.name) && p.deps.some((d) => hit.has(d))) {
        hit.add(p.name);
        grew = true;
      }
    }
  }
  return [...hit]
    .sort()
    .map((n) => `--filter ${n}`)
    .join(' ');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const base = process.argv[2] ?? 'origin/main';
  const out = execFileSync('git', ['diff', '--name-only', `${base}...HEAD`], { encoding: 'utf8' });
  console.log(affected(out.split('\n'), loadPackages(process.cwd())));
}
