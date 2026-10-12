#!/usr/bin/env node
// Sends a correctly signed GitHub webhook to the local API.
//   node tools/dev/replay-webhook/index.mjs --file payload.json --event pull_request [--tamper]
//     [--repeat N] [--redeliver] [--delivery-id UUID] [--secret-env NAME] [--url URL] [--expect-status N]
import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import { buildRequest } from './sign.mjs';

export function assertNotRealGithub(url) {
  if (/(^|\.)github\.com(\/|:|$)/i.test(new URL(url).hostname)) {
    throw new Error('refusing to send dev webhooks to a real GitHub host');
  }
}

async function main() {
  const { values } = parseArgs({
    options: {
      file: { type: 'string' },
      event: { type: 'string', default: 'pull_request' },
      url: { type: 'string', default: 'http://127.0.0.1:23000/api/v1/webhooks/github' },
      'secret-env': { type: 'string', default: 'GITHUB_WEBHOOK_SECRET' },
      'delivery-id': { type: 'string' },
      'expect-status': { type: 'string' },
      repeat: { type: 'string', default: '1' },
      tamper: { type: 'boolean', default: false },
      redeliver: { type: 'boolean', default: false },
    },
  });
  if (!values.file) throw new Error('--file <payload.json> is required');
  assertNotRealGithub(values.url);
  const secret = process.env[values['secret-env']];
  if (!secret) throw new Error(`env ${values['secret-env']} is not set`);
  const body = readFileSync(values.file);
  const baseId = values['delivery-id'] ?? randomUUID();
  const expected = values['expect-status'] ? Number(values['expect-status']) : undefined;
  let failed = false;
  for (let i = 0; i < Number(values.repeat); i++) {
    const deliveryId = values.redeliver && i > 0 ? randomUUID() : baseId;
    const req = buildRequest({ secret, event: values.event, body, deliveryId, tamper: values.tamper });
    const res = await fetch(values.url, { method: 'POST', headers: req.headers, body: req.body });
    console.log(`delivery=${deliveryId} status=${res.status} ${await res.text()}`);
    const ok = expected ? res.status === expected : res.ok;
    if (!ok) failed = true;
  }
  process.exit(failed ? 1 : 0);
}

if (process.argv[1]?.endsWith('index.mjs')) {
  main().catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
