import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseNgrokTunnels } from './tunnel.mjs';

const root = new URL('../../', import.meta.url);
const manifest = JSON.parse(readFileSync(new URL('infra/github/app-manifest.json', root), 'utf8'));

test('manifest_contains_no_write_contents_or_admin_permissions', () => {
  const p = manifest.default_permissions;
  assert.equal(p.contents, 'read');
  assert.ok(!('administration' in p));
  assert.ok(!Object.values(p).includes('admin'));
});

test('tunnel_script_parses_ngrok_local_api_response', () => {
  const url = parseNgrokTunnels({
    tunnels: [{ public_url: 'http://x.ngrok-free.app' }, { public_url: 'https://x.ngrok-free.app' }],
  });
  assert.equal(url, 'https://x.ngrok-free.app/api/v1/webhooks/github');
  assert.equal(parseNgrokTunnels({ tunnels: [] }), undefined);
});

test('gitignore_covers_pem_env_and_secrets_dir', () => {
  const g = readFileSync(new URL('.gitignore', root), 'utf8').split('\n');
  for (const entry of ['.env', '*.pem', '.secrets/']) assert.ok(g.includes(entry), entry);
});
