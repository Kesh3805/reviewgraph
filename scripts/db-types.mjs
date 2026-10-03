#!/usr/bin/env node
// Regenerates apps/api/src/db/generated.ts from the migrated schema (API-002, ADR-014).
//   node scripts/db-types.mjs            write the committed file
//   node scripts/db-types.mjs --check    regenerate to a temp file and fail on drift (CI-005)
//   --database-url <url>                 use an already migrated database instead of a throwaway one
//
// By default a throwaway postgres container is started on a random loopback port, every file in
// engine/migrations is applied in order (the same files `sqlx migrate run` applies), the types
// are generated, and the container is removed. The engine migrations stay the only schema source.
import { spawn, spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const API = path.join(ROOT, 'apps', 'api');
const OUT = path.join(API, 'src', 'db', 'generated.ts');
const MIGRATIONS = path.join(ROOT, 'engine', 'migrations');
const IMAGE = process.env.RG_POSTGRES_IMAGE ?? 'postgres:16.15-alpine';
const requireFromApi = createRequire(path.join(API, 'package.json'));

const args = process.argv.slice(2);
const check = args.includes('--check');
const urlFlag = args.indexOf('--database-url');
const givenUrl = urlFlag >= 0 ? args[urlFlag + 1] : undefined;

const normalizeEol = (text) => text.replace(/\r\n/g, '\n');

function docker(dockerArgs) {
  const res = spawnSync('docker', dockerArgs, { encoding: 'utf8' });
  if (res.status !== 0) throw new Error(`docker ${dockerArgs[0]} failed: ${res.stderr}`);
  return res.stdout.trim();
}

async function waitForPg(pg, url) {
  for (let attempt = 0; attempt < 60; attempt++) {
    const client = new pg.Client({ connectionString: url });
    try {
      await client.connect();
      await client.query('SELECT 1');
      await client.end();
      return;
    } catch {
      await client.end().catch(() => undefined);
      await new Promise((r) => setTimeout(r, 500));
    }
  }
  throw new Error('throwaway postgres did not become ready');
}

async function applyMigrations(pg, url) {
  const client = new pg.Client({ connectionString: url });
  await client.connect();
  try {
    const files = readdirSync(MIGRATIONS)
      .filter((f) => f.endsWith('.sql'))
      .sort();
    for (const file of files) {
      // Each file is one multi-statement simple query, which postgres runs in one transaction.
      await client.query(readFileSync(path.join(MIGRATIONS, file), 'utf8'));
    }
  } finally {
    await client.end();
  }
}

function codegen(url, outFile) {
  const bin = requireFromApi.resolve('kysely-codegen/dist/cli/bin.js');
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      [bin, '--dialect', 'postgres', '--url', url, '--out-file', outFile, '--singularize', 'false'],
      { cwd: API, stdio: 'inherit' },
    );
    child.on('error', reject);
    child.on('exit', (code) =>
      code === 0 ? resolve() : reject(new Error(`kysely-codegen exited ${code}`)),
    );
  });
}

async function main() {
  const pg = requireFromApi('pg');
  let container;
  let url = givenUrl;
  try {
    if (!url) {
      const password = 'rg-codegen';
      container = docker([
        'run',
        '-d',
        '--rm',
        '-p',
        '127.0.0.1::5432',
        '-e',
        `POSTGRES_PASSWORD=${password}`,
        '-e',
        'POSTGRES_DB=reviewgraph',
        IMAGE,
      ]);
      const mapping = docker(['port', container, '5432/tcp']).split('\n')[0];
      const port = mapping.slice(mapping.lastIndexOf(':') + 1);
      url = `postgres://postgres:${password}@127.0.0.1:${port}/reviewgraph`;
      await waitForPg(pg, url);
      await applyMigrations(pg, url);
    }
    const target = check
      ? path.join(mkdtempSync(path.join(tmpdir(), 'rg-dbtypes-')), 'generated.ts')
      : OUT;
    await codegen(url, target);
    if (check) {
      const fresh = normalizeEol(readFileSync(target, 'utf8'));
      const committed = normalizeEol(readFileSync(OUT, 'utf8'));
      rmSync(path.dirname(target), { recursive: true, force: true });
      if (fresh !== committed) {
        console.error(
          'apps/api/src/db/generated.ts is out of date: run `pnpm db:types` and commit.',
        );
        process.exitCode = 1;
      } else {
        console.log('db types are up to date');
      }
    } else {
      // Keep LF endings so the committed bytes match on every platform.
      writeFileSync(OUT, normalizeEol(readFileSync(OUT, 'utf8')));
    }
  } finally {
    if (container) spawnSync('docker', ['rm', '-f', container], { stdio: 'ignore' });
  }
}

main().catch((err) => {
  console.error(err.message);
  process.exit(1);
});
