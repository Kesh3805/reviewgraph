import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, createApiClient, toApiError } from '../lib/api-client';

describe('api client errors', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('api_error_maps_problem_json', async () => {
    const res = new Response(
      JSON.stringify({
        type: 'about:blank',
        title: 'forbidden',
        status: 403,
        detail: 'not a member',
        request_id: 'req-1',
      }),
      { status: 403, headers: { 'content-type': 'application/problem+json' } },
    );
    const err = await toApiError(res);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(403);
    expect(err.message).toBe('not a member');
    expect(err.requestId).toBe('req-1');
    expect(err.isUnauthorized).toBe(false);
  });

  it('tolerates non-json error bodies', async () => {
    const err = await toApiError(new Response('<html>bad gateway</html>', { status: 502 }));
    expect(err.status).toBe(502);
    expect(err.problem.status).toBe(502);
  });

  it('client throws ApiError on a 401 problem response', async () => {
    vi.stubGlobal(
      'fetch',
      async () =>
        new Response(JSON.stringify({ title: 'unauthorized', status: 401 }), {
          status: 401,
          headers: { 'content-type': 'application/problem+json' },
        }),
    );
    const client = createApiClient('http://api.test');
    await expect(client.GET('/api/v1/auth/me')).rejects.toMatchObject({
      name: 'ApiError',
      status: 401,
      isUnauthorized: true,
    });
  });
});
