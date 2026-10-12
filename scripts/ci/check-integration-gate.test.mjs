import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { budgetExceeded, quarantined, ungatedFiles } from './check-integration-gate.mjs';

function fixture(files) {
  const root = mkdtempSync(join(tmpdir(), 'gate-'));
  for (const [rel, body] of Object.entries(files)) {
    const p = join(root, rel);
    mkdirSync(join(p, '..'), { recursive: true });
    writeFileSync(p, body);
  }
  return root;
}

test('integration_feature_gate_guard flags an ungated database test', () => {
  const root = fixture({
    'crates/a/tests/bad.rs': 'fn t() { let u = std::env::var("DATABASE_URL").unwrap(); }',
    'crates/a/tests/good.rs':
      '#![cfg(feature = "integration")]\nfn t() { std::env::var("DATABASE_URL"); }',
    'crates/a/tests/common/pg.rs': 'fn h() { std::env::var("TEST_DATABASE_URL"); }',
    'crates/a/tests/literal.rs': 'const X: &str = "env:DATABASE_URL";',
  });
  assert.deepEqual(
    ungatedFiles(root).map((f) => f.replaceAll('\\', '/')),
    ['crates/a/tests/bad.rs'],
  );
});

test('quarantine_count_does_not_increase', () => {
  const root = fixture({
    'crates/a/tests/q.rs':
      '#[ignore = "quarantine:ISSUE-7"]\nfn t() {}\n#[ignore = "slow"]\nfn u() {}',
  });
  const q = quarantined(root);
  assert.equal(q.length, 1);
  assert.equal(q[0].issue, 'ISSUE-7');
  assert.equal(budgetExceeded(1, '0\n'), true);
  assert.equal(budgetExceeded(1, '1\n'), false);
});
