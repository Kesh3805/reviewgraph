#!/usr/bin/env node
// Drift check for the contracts pipeline.
//   node scripts/check.mjs [--schemas <dir>] [--generated <dir>] [--fresh <dir>]
// 1. Re-export the schemas from Rust (through the container) into .tmp/, unless --fresh
//    points at an already exported directory (used by tests to avoid the container).
// 2. Byte-compare with the committed schemas (--schemas, default schemas/).
// 3. Generate TS from the fresh schemas and byte-compare with src/generated (--generated).
// Exits 1 and lists every drifted file on any difference.
import { readdir, readFile, rm, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { generate } from './generate.mjs';

const PKG = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const ROOT = path.resolve(PKG, '..', '..');

function arg(flag, fallback) {
  const i = process.argv.indexOf(flag);
  return i >= 0 && process.argv[i + 1] ? path.resolve(process.argv[i + 1]) : fallback;
}

async function listFiles(dir) {
  try {
    return (await readdir(dir)).sort();
  } catch {
    return [];
  }
}

/** Returns human-readable drift descriptions for two directories. */
async function diffDirs(label, expectedDir, actualDir) {
  const drift = [];
  const names = [
    ...new Set([...(await listFiles(expectedDir)), ...(await listFiles(actualDir))]),
  ].sort();
  for (const name of names) {
    const [a, b] = await Promise.all(
      [expectedDir, actualDir].map((d) => readFile(path.join(d, name)).catch(() => null)),
    );
    if (a === null) drift.push(`${label}/${name}: unexpected file (not produced by the exporter)`);
    else if (b === null) drift.push(`${label}/${name}: missing`);
    else if (!a.equals(b)) drift.push(`${label}/${name}: content differs`);
  }
  return drift;
}

const schemasDir = arg('--schemas', path.join(PKG, 'schemas'));
const generatedDir = arg('--generated', path.join(PKG, 'src', 'generated'));
const tmp = path.join(PKG, '.tmp', `check-${process.pid}`);

try {
  await rm(tmp, { recursive: true, force: true });
  await mkdir(tmp, { recursive: true });

  let freshSchemas = arg('--fresh', null);
  if (!freshSchemas) {
    freshSchemas = path.join(tmp, 'schemas');
    const rel = path.relative(ROOT, freshSchemas).split(path.sep).join('/');
    const res = spawnSync(
      process.execPath,
      [
        path.join(ROOT, 'scripts', 'rg.mjs'),
        'cargo',
        'run',
        '-q',
        '-p',
        'review-cli',
        '--',
        'contracts',
        'export',
        '--out',
        `/repo/${rel}`,
      ],
      { cwd: ROOT, stdio: ['ignore', 'ignore', 'inherit'] },
    );
    if (res.status !== 0) {
      console.error('contracts export failed');
      process.exit(res.status ?? 1);
    }
  }

  const freshGenerated = path.join(tmp, 'generated');
  await generate(freshSchemas, freshGenerated);

  const drift = [
    ...(await diffDirs('schemas', freshSchemas, schemasDir)),
    ...(await diffDirs('src/generated', freshGenerated, generatedDir)),
  ];
  if (drift.length > 0) {
    console.error('contracts drift detected:');
    for (const d of drift) console.error(`  ${d}`);
    console.error('run: pnpm contracts:export && pnpm contracts:generate');
    process.exitCode = 1;
  } else {
    console.log('contracts are up to date');
  }
} finally {
  await rm(tmp, { recursive: true, force: true });
}
