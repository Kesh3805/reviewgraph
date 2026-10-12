import { test } from 'node:test';
import assert from 'node:assert/strict';
import { trivyIgnore } from './exceptions-to-trivyignore.mjs';

const TOML = `
[[exception]]
id = "CVE-2026-0001"
reason = "not reachable"
owner = "security"
added = "2026-09-01"
expires = "2026-11-01"

[[exception]]
id = "CVE-2026-0002"
reason = "old"
owner = "security"
added = "2026-01-01"
expires = "2026-02-01"
`;

test('trivyIgnore lists only unexpired exceptions', () => {
  const out = trivyIgnore(TOML, new Date('2026-10-12'));
  assert.match(out, /^CVE-2026-0001$/m);
  assert.doesNotMatch(out, /CVE-2026-0002/);
});

test('trivyIgnore tolerates an empty exceptions file', () => {
  assert.equal(trivyIgnore('', new Date()).trim().split('\n').length, 1);
});
