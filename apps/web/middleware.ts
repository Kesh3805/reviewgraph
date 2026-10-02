import { NextResponse, type NextRequest } from 'next/server';
import { SESSION_COOKIE } from './lib/constants';
import { buildCsp, generateNonce } from './lib/csp';

const PUBLIC_PATHS = ['/login'];

/** Only same-origin relative paths may be used as a post-login destination. */
export function safeNext(path: string): string | undefined {
  return path.startsWith('/') && !path.startsWith('//') && !path.startsWith('/login')
    ? path
    : undefined;
}

export function middleware(request: NextRequest): NextResponse {
  const nonce = generateNonce();
  const csp = buildCsp(nonce, process.env.NODE_ENV !== 'production');
  const { pathname, search } = request.nextUrl;

  const requestHeaders = new Headers(request.headers);
  // Next reads the nonce from the request CSP header and applies it to its own scripts.
  requestHeaders.set('x-nonce', nonce);
  requestHeaders.set('content-security-policy', csp);

  const isPublic = PUBLIC_PATHS.some((p) => pathname === p || pathname.startsWith(`${p}/`));
  let response: NextResponse;
  if (!isPublic && !request.cookies.has(SESSION_COOKIE)) {
    const url = request.nextUrl.clone();
    url.pathname = '/login';
    url.search = '';
    const next = safeNext(`${pathname}${search}`);
    if (next && next !== '/') url.searchParams.set('next', next);
    response = NextResponse.redirect(url);
  } else {
    response = NextResponse.next({ request: { headers: requestHeaders } });
  }
  response.headers.set('content-security-policy', csp);
  return response;
}

export const config = {
  // The API (proxied under /api) and static assets are not page navigations.
  matcher: ['/((?!api/|_next/static|_next/image|favicon.ico).*)'],
};
