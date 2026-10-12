#!/usr/bin/env node
// Starts `ngrok http 23000` and prints the webhook URL. The ngrok auth token is never read or printed.
import { spawn } from 'node:child_process';

export const WEBHOOK_PATH = '/api/v1/webhooks/github';

export function parseNgrokTunnels(json) {
  const t = (json.tunnels ?? []).find((x) => x.public_url?.startsWith('https://'));
  return t ? `${t.public_url}${WEBHOOK_PATH}` : undefined;
}

async function main() {
  const port = process.env.API_PORT ?? '23000';
  const args = ['http', port];
  if (process.env.NGROK_DOMAIN) args.push(`--domain=${process.env.NGROK_DOMAIN}`);
  const child = spawn('ngrok', args, { stdio: 'ignore' });
  child.on('error', () => {
    console.error(
      'ngrok not found: install from https://ngrok.com/download and run `ngrok config add-authtoken <token>`',
    );
    process.exit(1);
  });
  for (let i = 0; i < 30; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    try {
      const url = parseNgrokTunnels(
        await (await fetch('http://127.0.0.1:4040/api/tunnels')).json(),
      );
      if (url) {
        console.log(
          `Webhook URL: ${url}\nUpdate the GitHub App webhook URL to this value. Ctrl+C stops the tunnel.`,
        );
        return;
      }
    } catch {
      /* ngrok not up yet */
    }
  }
  console.error('tunnel did not come up; is the API running (pnpm dev:up:app)?');
  child.kill();
  process.exit(1);
}

if (process.argv[1]?.endsWith('tunnel.mjs')) main();
