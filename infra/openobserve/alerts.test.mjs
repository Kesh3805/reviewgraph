import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { catalogNames, validate } from './validate.mjs';

const ROOT = dirname(fileURLToPath(import.meta.url));
const alerts = readdirSync(join(ROOT, 'alerts'))
  .filter((f) => f.endsWith('.json'))
  .map((f) => JSON.parse(readFileSync(join(ROOT, 'alerts', f), 'utf8')));
const byName = Object.fromEntries(alerts.map((a) => [a.name, a]));

test('every_alert_has_runbook_and_vice_versa', () => {
  assert.deepEqual(validate(), []);
  assert.equal(alerts.length, 10);
});

test('alert_thresholds_match_master_plan_table', () => {
  const expect = {
    'rg-review-latency-high': [120, '15m'],
    'rg-dead-jobs': [0, '0m'],
    'rg-webhook-signature-failures': [10, '5m'],
    'rg-model-error-rate': [0.05, '15m'],
    'rg-queue-wait-high': [60, '10m'],
    'rg-qdrant-latency-high': [0.5, '10m'],
    'rg-pg-connection-saturation': [0.8, '5m'],
    'rg-false-positive-rate-high': [0.15, '0m'],
    'rg-qdrant-scope-violation': [0, '0m'],
    'rg-tenancy-denied-spike': [50, '5m'],
  };
  for (const [name, [value, dur]] of Object.entries(expect)) {
    assert.equal(byName[name].threshold.value, value, name);
    assert.equal(byName[name].for, dur, name);
  }
  assert.equal(byName['rg-qdrant-scope-violation'].severity, 'page');
  assert.equal(byName['rg-tenancy-denied-spike'].severity, 'page');
});

test('alert_queries_reference_known_metrics', () => {
  const names = catalogNames();
  for (const a of alerts) assert.ok(names.has(a.metric), `${a.name}: ${a.metric}`);
});

test('destination_secrets_not_in_committed_files', () => {
  for (const dir of ['destinations', 'templates']) {
    for (const f of readdirSync(join(ROOT, dir))) {
      const text = readFileSync(join(ROOT, dir, f), 'utf8');
      assert.doesNotMatch(text, /https?:\/\/(?!localhost|127\.0\.0\.1)/, f);
    }
  }
});

test('runbooks_have_required_headings', () => {
  assert.deepEqual(
    validate().filter((e) => e.includes('heading')),
    [],
  );
});
