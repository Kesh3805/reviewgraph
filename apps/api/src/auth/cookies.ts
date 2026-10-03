import type { CookieOptions, Request, Response } from 'express';

export const SESSION_COOKIE = 'rg_session';
export const CSRF_COOKIE = 'rg_csrf';
export const OAUTH_STATE_COOKIE = 'rg_oauth_state';
export const CSRF_HEADER = 'x-csrf-token';

/** 12 hours, in seconds. */
export const SESSION_TTL_SECONDS = 12 * 60 * 60;

/** Parses the `Cookie` header. Malformed pairs are skipped; values are URI-decoded. */
export function parseCookies(header: string | undefined): Record<string, string> {
  const out: Record<string, string> = {};
  if (!header) return out;
  for (const part of header.split(';')) {
    const eq = part.indexOf('=');
    if (eq < 1) continue;
    const name = part.slice(0, eq).trim();
    let value = part.slice(eq + 1).trim();
    if (value.startsWith('"') && value.endsWith('"')) value = value.slice(1, -1);
    try {
      out[name] ??= decodeURIComponent(value);
    } catch {
      // A value that is not valid percent-encoding is ignored rather than trusted raw.
    }
  }
  return out;
}

export function cookiesOf(req: Request): Record<string, string> {
  return parseCookies(req.headers.cookie);
}

/** HttpOnly + SameSite=Lax + Path=/; `Secure` everywhere except local development. */
export function cookieOptions(
  nodeEnv: string,
  overrides: Partial<CookieOptions> = {},
): CookieOptions {
  return {
    httpOnly: true,
    secure: nodeEnv !== 'development',
    sameSite: 'lax',
    path: '/',
    ...overrides,
  };
}

export function clearCookie(res: Response, name: string, options: CookieOptions): void {
  res.clearCookie(name, { ...options, maxAge: undefined });
}
