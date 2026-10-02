export const CSRF_COOKIE_NAME = 'rg_csrf';
export const CSRF_HEADER_NAME = 'X-CSRF-Token';

const SAFE_METHODS = new Set(['GET', 'HEAD', 'OPTIONS']);

export function isMutation(method: string): boolean {
  return !SAFE_METHODS.has(method.toUpperCase());
}

/** Reads the CSRF token from a `document.cookie`-style string. */
export function readCsrfToken(cookieString: string): string | undefined {
  for (const part of cookieString.split(';')) {
    const [name, ...rest] = part.trim().split('=');
    if (name === CSRF_COOKIE_NAME) return decodeURIComponent(rest.join('='));
  }
  return undefined;
}

/** Sets `X-CSRF-Token` on non-GET requests. Safe methods are returned untouched. */
export function withCsrf(
  method: string,
  headers: Headers,
  cookieString = typeof document === 'undefined' ? '' : document.cookie,
): Headers {
  if (!isMutation(method)) return headers;
  const token = readCsrfToken(cookieString);
  if (token) headers.set(CSRF_HEADER_NAME, token);
  return headers;
}
