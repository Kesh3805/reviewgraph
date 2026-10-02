import { NextRequest } from 'next/server';
import { describe, expect, it } from 'vitest';
import { middleware, safeNext } from '../middleware';

const req = (path: string, cookie?: string) =>
  new NextRequest(`http://localhost:3000${path}`, cookie ? { headers: { cookie } } : undefined);

describe('middleware', () => {
  it('middleware_redirects_without_session', () => {
    const res = middleware(req('/repositories?tab=open'));
    expect(res.status).toBe(307);
    const location = new URL(res.headers.get('location') ?? '');
    expect(location.pathname).toBe('/login');
    expect(location.searchParams.get('next')).toBe('/repositories?tab=open');
  });

  it('lets authenticated requests through', () => {
    const res = middleware(req('/', 'rg_session=abc'));
    expect(res.status).toBe(200);
    expect(res.headers.get('location')).toBeNull();
  });

  it('keeps /login public', () => {
    expect(middleware(req('/login')).status).toBe(200);
  });

  it('sets a strict nonce based CSP', () => {
    const csp = middleware(req('/login')).headers.get('content-security-policy') ?? '';
    expect(csp).toContain("default-src 'self'");
    expect(csp).toMatch(/script-src 'self' 'nonce-[^']+' 'strict-dynamic'/);
    expect(csp).toContain("object-src 'none'");
    expect(csp).toContain("frame-ancestors 'none'");
  });

  it('only allows same-origin relative next paths', () => {
    expect(safeNext('//evil.example')).toBeUndefined();
    expect(safeNext('https://evil.example')).toBeUndefined();
    expect(safeNext('/login')).toBeUndefined();
    expect(safeNext('/repositories')).toBe('/repositories');
  });
});
