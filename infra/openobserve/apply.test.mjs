import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { apply } from './apply.mjs';
import { validate } from './validate.mjs';

const dash = (id, extra = {}) => ({ dashboardId: id, title: id, ...extra });

function fixtureDir(dashboards) {
  const dir = mkdtempSync(join(tmpdir(), 'oo-'));
  mkdirSync(join(dir, 'dashboards'));
  for (const d of dashboards) writeFileSync(join(dir, 'dashboards', `${d.dashboardId}.json`), JSON.stringify(d));
  return dir;
}

// Minimal in-memory OpenObserve double.
function mockServer(initial = []) {
  const store = new Map(initial.map((d) => [d.dashboardId, d]));
  const writes = [];
  const fetchImpl = async (url, { method, body }) => {
    const path = new URL(url).pathname.replace('/api/default/', '');
    const json = (o, status = 200) => ({ ok: status < 300, status, json: async () => o, text: async () => JSON.stringify(o) });
    if (method === 'GET') return json({ dashboards: [...store.values()] });
    writes.push(method);
    if (method === 'POST') store.set(JSON.parse(body).dashboardId, JSON.parse(body));
    if (method === 'PUT') store.set(decodeURIComponent(path.split('/')[1]), JSON.parse(body));
    if (method === 'DELETE') store.delete(decodeURIComponent(path.split('/')[1]));
    return json({});
  };
  return { fetchImpl, writes, store };
}

const opts = (dir, srv, extra = {}) => ({ baseUrl: 'http://localhost:25080', baseDir: dir, fetchImpl: srv.fetchImpl, ...extra });

test('apply_creates_missing_dashboards', async () => {
  const srv = mockServer();
  const r = await apply(opts(fixtureDir([dash('rg-a'), dash('rg-b')]), srv));
  assert.equal(r.plan.create.length, 2);
  assert.equal(srv.store.size, 2);
});

test('apply_updates_changed_and_skips_identical', async () => {
  const srv = mockServer([dash('rg-a'), dash('rg-b', { title: 'old' })]);
  const r = await apply(opts(fixtureDir([dash('rg-a', { updatedAt: 'x' }), dash('rg-b')]), srv));
  assert.equal(r.plan.skip.length, 1);
  assert.equal(r.plan.update.length, 1);
  const again = await apply(opts(fixtureDir([dash('rg-a'), dash('rg-b')]), srv));
  assert.equal(again.changes, 0);
});

test('apply_dry_run_makes_no_writes', async () => {
  const srv = mockServer();
  const r = await apply(opts(fixtureDir([dash('rg-a')]), srv, { dryRun: true }));
  assert.equal(r.changes, 1);
  assert.deepEqual(srv.writes, []);
});

test('prune_only_removes_rg_prefixed_dashboards', async () => {
  const srv = mockServer([dash('rg-old'), dash('manual')]);
  await apply(opts(fixtureDir([dash('rg-a')]), srv, { prune: true }));
  assert.ok(!srv.store.has('rg-old'));
  assert.ok(srv.store.has('manual'));
});

test('apply_refuses_remote_endpoint_without_flag', async () => {
  const srv = mockServer();
  await assert.rejects(apply(opts(fixtureDir([]), srv, { baseUrl: 'https://oo.example.com' })), /non-loopback/);
});

test('validate_accepts_all_committed_dashboards', () => {
  assert.deepEqual(validate(), []);
});

test('validate_rejects_unknown_metric_reference', () => {
  const dir = fixtureDir([
    dash('rg-x', { tabs: [{ panels: [{ queries: [{ fields: { stream: 'nope_total', stream_type: 'metrics' } }] }] }] }),
  ]);
  assert.match(validate({ baseDir: dir }).join('\n'), /unknown metric nope_total/);
});

test('validate_rejects_duplicate_dashboard_id', () => {
  const dir = fixtureDir([dash('rg-x')]);
  writeFileSync(join(dir, 'dashboards', 'copy.json'), JSON.stringify(dash('rg-x')));
  assert.match(validate({ baseDir: dir }).join('\n'), /duplicate dashboardId/);
});
