import { afterEach, describe, expect, it, vi } from 'vitest';
import { createApiClient } from '../lib/api-client';
import { readCsrfToken, withCsrf } from '../lib/csrf';

describe('csrf', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('reads the token from the cookie string', () => {
    expect(readCsrfToken('a=1; rg_csrf=tok%3D1; b=2')).toBe('tok=1');
    expect(readCsrfToken('a=1')).toBeUndefined();
  });

  it('csrf_header_set_on_mutation', () => {
    const cookie = 'rg_csrf=abc123';
    for (const method of ['POST', 'PUT', 'PATCH', 'DELETE']) {
      expect(withCsrf(method, new Headers(), cookie).get('X-CSRF-Token')).toBe('abc123');
    }
    for (const method of ['GET', 'HEAD', 'OPTIONS']) {
      expect(withCsrf(method, new Headers(), cookie).has('X-CSRF-Token')).toBe(false);
    }
  });

  it('api client injects the header on POST but not GET', async () => {
    const seen: Request[] = [];
    vi.stubGlobal('document', { cookie: 'rg_csrf=from-cookie' });
    vi.stubGlobal('fetch', async (req: Request) => {
      seen.push(req);
      return new Response(null, { status: 204 });
    });
    const client = createApiClient('http://api.test');
    await client.POST('/api/v1/auth/logout');
    expect(seen[0]?.headers.get('x-csrf-token')).toBe('from-cookie');
  });
});
