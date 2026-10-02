import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const GENERATED = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '..',
  'src',
  'generated',
);

// INV-011 / INV-012 at the contract boundary: the TypeScript publisher can only name COMMENT.
test('contracts_review_event_ts_is_comment_literal', () => {
  const ts = readFileSync(path.join(GENERATED, 'ReviewEvent.ts'), 'utf8');
  assert.match(ts, /export type ReviewEvent = "COMMENT";/);
  for (const forbidden of ['APPROVE', 'REQUEST_CHANGES', 'MERGE']) {
    assert.ok(!ts.includes(forbidden), `${forbidden} must not appear in ReviewEvent.ts`);
  }
});

test('check conclusion is exactly success or neutral', () => {
  const ts = readFileSync(path.join(GENERATED, 'CheckConclusion.ts'), 'utf8');
  assert.match(ts, /export type CheckConclusion = "success" \| "neutral";/);
});
