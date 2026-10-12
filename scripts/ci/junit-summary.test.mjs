import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { parseJunit, summarize } from './junit-summary.mjs';

const XML = `<?xml version="1.0"?>
<testsuites tests="3" failures="1">
<testsuite name="a" tests="3">
<testcase classname="a" name="fast" time="0.010"/>
<testcase classname="a" name="slow" time="2.500"/>
<testcase classname="a" name="bad" time="0.100"><failure message="x">boom</failure></testcase>
<testcase classname="a" name="skip" time="0"><skipped/></testcase>
</testsuite></testsuites>`;

test('parseJunit counts tests, failures and skips', () => {
  const r = parseJunit(XML);
  assert.equal(r.tests, 4);
  assert.equal(r.failures, 1);
  assert.equal(r.skipped, 1);
});

test('summarize lists the slowest test first and tolerates an empty directory', () => {
  const dir = mkdtempSync(join(tmpdir(), 'junit-'));
  writeFileSync(join(dir, 'api.xml'), XML);
  const out = summarize(dir);
  assert.match(out, /\| api\.xml \| 4 \| 1 \| 1 \|/);
  assert.ok(out.indexOf('api.xml: slow') < out.indexOf('api.xml: fast'));
  assert.match(summarize(join(dir, 'missing')), /no JUnit files/);
});
