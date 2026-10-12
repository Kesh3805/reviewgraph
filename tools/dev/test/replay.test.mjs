import test from 'node:test';
import assert from 'node:assert/strict';
import { createHmac } from 'node:crypto';
import { buildRequest, sign } from '../replay-webhook/sign.mjs';
import { assertNotRealGithub } from '../replay-webhook/index.mjs';

test('signature_matches_reference_vector', () => {
  // Reference vector from GitHub's webhook validation docs.
  assert.equal(
    sign("It's a Secret to Everybody", 'Hello, World!'),
    'sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17',
  );
});

test('tamper_flag_breaks_signature', () => {
  const r = buildRequest({ secret: 's', event: 'ping', body: '{"a":1}', tamper: true });
  const expected = `sha256=${createHmac('sha256', 's').update('{"a":1}').digest('hex')}`;
  assert.equal(r.headers['x-hub-signature-256'], expected);
  assert.notEqual(sign('s', r.body), expected);
});

test('repeat_flag_sends_same_delivery_id', () => {
  const a = buildRequest({ secret: 's', event: 'ping', body: '{}', deliveryId: 'id-1' });
  const b = buildRequest({ secret: 's', event: 'ping', body: '{}', deliveryId: 'id-1' });
  assert.equal(a.headers['x-github-delivery'], b.headers['x-github-delivery']);
});

test('tools_refuse_real_github_base_url', () => {
  assert.throws(() => assertNotRealGithub('https://api.github.com/x'), /real GitHub/);
  assert.doesNotThrow(() => assertNotRealGithub('http://127.0.0.1:23000/x'));
});
