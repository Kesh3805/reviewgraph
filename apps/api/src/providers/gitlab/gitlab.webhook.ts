import { matchesAnySecret, headerValue } from '../common/secret-compare';
import type { HeaderBag, WebhookVerification } from '../ports';

/**
 * Verifies a GitLab webhook: the shared secret arrives verbatim in `X-Gitlab-Token` (GitLab
 * does not sign the body), so it is compared in constant time against every configured secret
 * (current and previous, for rotation). The event UUID is the idempotency key.
 */
export function verifyGitlabWebhook(
  secrets: readonly string[],
  headers: HeaderBag,
): WebhookVerification {
  const token = headerValue(headers, 'x-gitlab-token');
  if (!token) return { valid: false, reason: 'missing_signature' };
  // No configured secret means nothing can match: reject rather than accept unauthenticated.
  if (secrets.length === 0 || !matchesAnySecret(secrets, token)) {
    return { valid: false, reason: 'bad_signature' };
  }
  const deliveryId = headerValue(headers, 'x-gitlab-event-uuid');
  const eventName = headerValue(headers, 'x-gitlab-event');
  if (!deliveryId || !eventName) return { valid: false, reason: 'missing_headers' };
  return { valid: true, deliveryId, eventName };
}
