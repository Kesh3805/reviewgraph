import { createHmac, randomUUID } from 'node:crypto';

/** sha256=<hex HMAC-SHA256 of the exact raw body bytes>. */
export function sign(secret, body) {
  return `sha256=${createHmac('sha256', secret).update(body).digest('hex')}`;
}

/** Builds signed webhook headers. The body is never re-serialized after signing. */
export function buildRequest({ secret, event, body, deliveryId = randomUUID(), tamper = false }) {
  const raw = Buffer.from(body);
  const signature = sign(secret, raw);
  let sent = raw;
  if (tamper) {
    sent = Buffer.from(raw);
    sent[0] = sent[0] ^ 0x01;
  }
  return {
    body: sent,
    headers: {
      'content-type': 'application/json',
      'user-agent': 'GitHub-Hookshot/reviewgraph-dev',
      'x-github-event': event,
      'x-github-delivery': deliveryId,
      'x-github-hook-id': '1',
      'x-hub-signature-256': signature,
    },
  };
}
