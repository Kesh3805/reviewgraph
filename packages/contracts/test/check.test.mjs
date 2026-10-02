import test from 'node:test';
import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const PKG = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CHECK = path.join(PKG, 'scripts', 'check.mjs');
const SCHEMAS = path.join(PKG, 'schemas');

// `--fresh` stands in for a fresh Rust export so these tests do not need the build container.
const run = (schemas) =>
  spawnSync(process.execPath, [CHECK, '--schemas', schemas, '--fresh', SCHEMAS], {
    encoding: 'utf8',
  });

test('check passes when schemas match the exporter output', () => {
  const res = run(SCHEMAS);
  assert.equal(res.status, 0, res.stderr);
});

test('check detects drift', () => {
  const tmp = mkdtempSync(path.join(os.tmpdir(), 'rg-contracts-test-'));
  try {
    cpSync(SCHEMAS, tmp, { recursive: true });
    const file = path.join(tmp, 'SchemaInfo.schema.json');
    writeFileSync(file, readFileSync(file, 'utf8').replace('"object"', '"object" '));
    const res = run(tmp);
    assert.equal(res.status, 1);
    assert.match(res.stderr, /SchemaInfo\.schema\.json: content differs/);
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
});
