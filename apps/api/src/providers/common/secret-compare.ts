import { createHash, timingSafeEqual } from 'node:crypto';

/**
 * Constant-time string equality. Both sides are hashed first so `timingSafeEqual` always sees
 * equal-length input and the secret's length is not observable.
 */
export function constantTimeEquals(a: string, b: string): boolean {
  const da = createHash('sha256').update(a).digest();
  const db = createHash('sha256').update(b).digest();
  return timingSafeEqual(da, db);
}

/** True when `provided` equals ANY of `secrets`; every secret is compared (no short circuit). */
export function matchesAnySecret(secrets: readonly string[], provided: string): boolean {
  let ok = false;
  for (const secret of secrets) {
    if (constantTimeEquals(secret, provided)) ok = true;
  }
  return ok;
}

/** First value of a header from a Node-style header bag. */
export function headerValue(
  headers: Record<string, string | string[] | undefined>,
  name: string,
): string | undefined {
  const v = headers[name.toLowerCase()];
  return Array.isArray(v) ? v[0] : v;
}
