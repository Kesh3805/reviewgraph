/**
 * Strict Content-Security-Policy. Scripts are limited to same-origin and the per-request Next
 * nonce (`strict-dynamic` lets nonced scripts load their chunks); there are no inline scripts
 * apart from the nonced ones Next and the theme provider emit.
 */
export function buildCsp(nonce: string, isDev = false): string {
  const directives: Record<string, string[]> = {
    'default-src': ["'self'"],
    'script-src': [
      "'self'",
      `'nonce-${nonce}'`,
      "'strict-dynamic'",
      ...(isDev ? ["'unsafe-eval'"] : []),
    ],
    // Tailwind and Next inject style tags; styles cannot execute code.
    'style-src': ["'self'", "'unsafe-inline'"],
    'img-src': ["'self'", 'data:', 'https://avatars.githubusercontent.com'],
    'font-src': ["'self'"],
    'connect-src': ["'self'", ...(isDev ? ['ws:', 'wss:'] : [])],
    'object-src': ["'none'"],
    'base-uri': ["'self'"],
    'form-action': ["'self'"],
    'frame-ancestors': ["'none'"],
  };
  return Object.entries(directives)
    .map(([name, values]) => `${name} ${values.join(' ')}`)
    .join('; ');
}

export function generateNonce(): string {
  return btoa(crypto.randomUUID());
}
