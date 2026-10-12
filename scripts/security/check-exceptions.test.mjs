import assert from 'node:assert/strict';
import test from 'node:test';
import { checkExceptions, checkThreatModel, parseExceptions } from './check-exceptions.mjs';

const entry = (o) =>
  `[[exception]]\n${Object.entries(o)
    .map(([k, v]) => `${k} = "${v}"`)
    .join('\n')}\n`;
const good = {
  id: 'RUSTSEC-1',
  reason: 'unreachable',
  owner: 'sec',
  added: '2026-01-01',
  expires: '2026-02-01',
};
const now = new Date('2026-01-15');

test('exceptions_file_parses_and_rejects_expired_entries', () => {
  assert.equal(parseExceptions(entry(good)).length, 1);
  assert.deepEqual(checkExceptions(entry(good), now), []);
  assert.match(checkExceptions(entry(good), new Date('2026-03-01')).join(), /expired/);
  assert.match(checkExceptions(entry({ ...good, expires: '2026-12-01' }), now).join(), /90 days/);
});

test('exceptions_require_owner_and_reason', () => {
  const { owner, reason, ...rest } = good;
  const errs = checkExceptions(entry(rest), now).join();
  assert.match(errs, /missing owner/);
  assert.match(errs, /missing reason/);
});

test('threat_model_task_references_resolve', () => {
  const plan = '**Task ID:** SEC-001\n**Task ID:** SEC-002';
  assert.deepEqual(checkThreatModel('| T-001 | S | 1 | x | SEC-001 | t | L |', plan), []);
  assert.match(checkThreatModel('| T-001 | S | 1 | x | SEC-999 | t | L |', plan).join(), /SEC-999/);
});

test('threat_ids_unique', () => {
  const row = '| T-001 | S | 1 | x | SEC-001 | t | L |\n';
  assert.match(checkThreatModel(row + row, '**Task ID:** SEC-001').join(), /duplicate/);
});
