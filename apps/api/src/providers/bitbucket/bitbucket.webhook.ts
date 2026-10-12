import { verifyWebhookSignature } from '../../webhooks/signature';
import { headerValue } from '../common/secret-compare';
import type { HeaderBag, WebhookVerification } from '../ports';

/**
 * Verifies a Bitbucket webhook: `X-Hub-Signature` carries `sha256=<hex HMAC-SHA256 of the raw
 * body>`. A signature is MANDATORY: with no secret configured nothing verifies (rejected by
 * default), and an unsigned delivery is rejected even when a secret exists. The comparison is
 * constant time over every configured secret (rotation). `X-Request-UUID` is the idempotency
 * key; retries reuse it and only bump `X-Attempt-Number`.
 */
export function verifyBitbucketWebhook(
  secrets: readonly string[],
  headers: HeaderBag,
  rawBody: Buffer,
): WebhookVerification {
  const signature = headerValue(headers, 'x-hub-signature');
  if (!signature) return { valid: false, reason: 'missing_signature' };
  if (secrets.length === 0) return { valid: false, reason: 'bad_signature' };
  if (verifyWebhookSignature(secrets, rawBody, signature) !== 'valid') {
    return { valid: false, reason: 'bad_signature' };
  }
  const deliveryId = headerValue(headers, 'x-request-uuid');
  const eventName = headerValue(headers, 'x-event-key');
  if (!deliveryId || !eventName) return { valid: false, reason: 'missing_headers' };
  return { valid: true, deliveryId, eventName };
}
