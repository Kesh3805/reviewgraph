import { createHmac, timingSafeEqual } from 'node:crypto';

const SIGNATURE_PATTERN = /^sha256=([0-9a-fA-F]{64})$/;
const DIGEST_BYTES = 32;

export type SignatureResult = 'valid' | 'missing' | 'malformed' | 'mismatch';

/**
 * Verifies `X-Hub-Signature-256` (`sha256=` + hex HMAC-SHA256 of the RAW body).
 *
 * - The HMAC is computed over the exact bytes received, never over re-serialized JSON.
 * - A header whose decoded length is not 32 bytes is rejected before any comparison, so
 *   `timingSafeEqual` (which throws on unequal lengths) is only reached with equal-length input.
 * - Every configured secret is checked without short-circuiting, so rotation
 *   (`GITHUB_WEBHOOK_SECRET`, `GITHUB_WEBHOOK_SECRET_PREVIOUS`) leaks nothing about which matched.
 */
export function verifyWebhookSignature(
  secrets: readonly string[],
  rawBody: Buffer,
  header: string | undefined,
): SignatureResult {
  if (!header) return 'missing';
  const match = SIGNATURE_PATTERN.exec(header);
  if (!match?.[1]) return 'malformed';
  const provided = Buffer.from(match[1], 'hex');
  if (provided.length !== DIGEST_BYTES) return 'malformed';

  let ok = false;
  for (const secret of secrets) {
    const expected = createHmac('sha256', secret).update(rawBody).digest();
    if (timingSafeEqual(expected, provided)) ok = true;
  }
  return ok ? 'valid' : 'mismatch';
}

/** Computes the header value GitHub would send (tests and the signed replay tool). */
export function signWebhookBody(secret: string, rawBody: Buffer): string {
  return `sha256=${createHmac('sha256', secret).update(rawBody).digest('hex')}`;
}
