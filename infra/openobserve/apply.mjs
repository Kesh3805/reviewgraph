#!/usr/bin/env node
// Idempotently applies dashboards, alerts, destinations and templates to OpenObserve.
// Usage: node infra/openobserve/apply.mjs [--dry-run] [--prune] [--allow-remote]
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = dirname(fileURLToPath(import.meta.url));
const PREFIX = 'rg-';
const IGNORED = new Set(['updatedAt', 'updated_at', 'owner', 'created', 'createdAt', 'hash']);

// kind -> { dir, api path, key extractor for local and remote items, list unwrapper }
export const KINDS = [
  {
    kind: 'templates',
    dir: 'templates',
    path: 'alerts/templates',
    key: (o) => o.name,
    unwrap: (r) => r,
  },
  {
    kind: 'destinations',
    dir: 'destinations',
    path: 'alerts/destinations',
    key: (o) => o.name,
    unwrap: (r) => r,
  },
  {
    kind: 'dashboards',
    dir: 'dashboards',
    path: 'dashboards',
    key: (o) => o.dashboardId,
    unwrap: (r) => r.dashboards ?? r,
  },
  { kind: 'alerts', dir: 'alerts', path: 'alerts', key: (o) => o.name, unwrap: (r) => r.list ?? r },
];

export function normalize(obj) {
  if (Array.isArray(obj)) return obj.map(normalize);
  if (obj && typeof obj === 'object') {
    return Object.fromEntries(
      Object.keys(obj)
        .filter((k) => !IGNORED.has(k))
        .sort()
        .map((k) => [k, normalize(obj[k])]),
    );
  }
  return obj;
}

export const hashOf = (o) =>
  createHash('sha256')
    .update(JSON.stringify(normalize(o)))
    .digest('hex');

export function isLoopback(url) {
  const host = new URL(url).hostname;
  return ['localhost', '127.0.0.1', '[::1]', '::1'].includes(host);
}

function loadLocal(baseDir, spec) {
  const dir = join(baseDir, spec.dir);
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => f.endsWith('.json'))
    .sort()
    .map((f) => JSON.parse(readFileSync(join(dir, f), 'utf8')));
}

export async function apply({
  baseUrl,
  org = 'default',
  email = '',
  password = '',
  baseDir = ROOT,
  dryRun = false,
  prune = false,
  allowRemote = false,
  fetchImpl = fetch,
  log = () => {},
}) {
  if (!allowRemote && !isLoopback(baseUrl)) {
    throw new Error('refusing non-loopback endpoint without --allow-remote');
  }
  const headers = {
    'content-type': 'application/json',
    authorization: `Basic ${Buffer.from(`${email}:${password}`).toString('base64')}`,
  };
  const call = async (method, path, body) => {
    const res = await fetchImpl(`${baseUrl}/api/${org}/${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`${method} ${path} failed: ${res.status} ${text.slice(0, 200)}`);
    }
    return res.status === 204 ? null : res.json().catch(() => null);
  };

  const plan = { create: [], update: [], skip: [], delete: [] };
  for (const spec of KINDS) {
    const local = loadLocal(baseDir, spec);
    if (local.length === 0 && !prune) continue;
    const remoteRaw = await call('GET', spec.path);
    const remote = new Map(
      [spec.unwrap(remoteRaw)]
        .flat()
        .filter((o) => o && typeof o === 'object')
        .map((o) => [spec.key(o) ?? o.dashboard_id ?? o.v5?.dashboardId, o]),
    );
    const localKeys = new Set();
    for (const item of local) {
      const key = spec.key(item);
      localKeys.add(key);
      const existing = remote.get(key);
      if (!existing) plan.create.push({ spec, key, item });
      else if (hashOf(existing) === hashOf(item)) plan.skip.push({ spec, key });
      else plan.update.push({ spec, key, item });
    }
    if (prune) {
      for (const [key, o] of remote) {
        if (typeof key === 'string' && key.startsWith(PREFIX) && !localKeys.has(key)) {
          plan.delete.push({ spec, key, o });
        }
      }
    }
  }

  const changes = plan.create.length + plan.update.length + plan.delete.length;
  log(
    `plan: ${plan.create.length} create, ${plan.update.length} update, ${plan.delete.length} delete, ${plan.skip.length} unchanged`,
  );
  if (dryRun) return { plan, changes, applied: false };
  for (const { spec, key, item } of plan.create)
    await call('POST', spec.path, item).catch(wrap(spec, key));
  for (const { spec, key, item } of plan.update)
    await call('PUT', `${spec.path}/${encodeURIComponent(key)}`, item).catch(wrap(spec, key));
  for (const { spec, key } of plan.delete)
    await call('DELETE', `${spec.path}/${encodeURIComponent(key)}`).catch(wrap(spec, key));
  return { plan, changes, applied: true };
}

const wrap = (spec, key) => (e) => {
  throw new Error(`${spec.kind} ${key}: ${e.message}`);
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const args = new Set(process.argv.slice(2));
  const endpoint =
    process.env.RG_OO_URL ?? process.env.OTEL_EXPORTER_OTLP_ENDPOINT ?? 'http://localhost:25080';
  const baseUrl = new URL(endpoint).origin;
  try {
    const { changes } = await apply({
      baseUrl,
      email: process.env.RG_OO_EMAIL,
      password: process.env.RG_OO_PASSWORD,
      dryRun: args.has('--dry-run'),
      prune: args.has('--prune'),
      allowRemote: args.has('--allow-remote'),
      log: console.log,
    });
    console.log(`${changes} changes`);
  } catch (e) {
    console.error(e.message);
    process.exit(1);
  }
}
