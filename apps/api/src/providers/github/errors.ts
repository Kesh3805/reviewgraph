import { ProviderError } from '../ports';

interface RequestErrorLike {
  status: number;
  message?: string;
  response?: { headers?: Record<string, string | number | undefined>; data?: unknown };
}

function isRequestError(err: unknown): err is RequestErrorLike {
  return (
    typeof err === 'object' && err !== null && typeof (err as RequestErrorLike).status === 'number'
  );
}

function retryAfterMs(err: RequestErrorLike): number | undefined {
  const headers = err.response?.headers ?? {};
  const retryAfter = Number(headers['retry-after']);
  if (Number.isFinite(retryAfter) && retryAfter > 0) return retryAfter * 1000;
  if (headers['x-ratelimit-remaining'] === '0' || headers['x-ratelimit-remaining'] === 0) {
    const reset = Number(headers['x-ratelimit-reset']);
    if (Number.isFinite(reset)) return Math.max(0, reset * 1000 - Date.now());
  }
  return undefined;
}

/**
 * Maps an Octokit failure to the provider-neutral `ProviderError`. The message carries only the
 * operation and status; request headers (credentials) never enter it.
 */
export function toProviderError(err: unknown, operation: string): ProviderError {
  if (err instanceof ProviderError) return err;
  if (!isRequestError(err)) {
    return new ProviderError('transient', `${operation} failed (network error)`);
  }
  const msg = `${operation} failed (status ${err.status})`;
  const waitMs = retryAfterMs(err);
  const headers = err.response?.headers ?? {};
  const limited =
    err.status === 429 ||
    (err.status === 403 &&
      (headers['x-ratelimit-remaining'] === '0' ||
        headers['x-ratelimit-remaining'] === 0 ||
        waitMs !== undefined ||
        /rate limit|abuse/i.test(
          String((err.response?.data as { message?: string })?.message ?? ''),
        )));
  if (limited) return new ProviderError('rate_limited', msg, { retryAfterMs: waitMs });
  if (err.status === 404) return new ProviderError('not_found', msg);
  if (err.status === 401 || err.status === 403) return new ProviderError('forbidden', msg);
  if (err.status === 400 || err.status === 409 || err.status === 410 || err.status === 422) {
    return new ProviderError('invalid', msg);
  }
  return new ProviderError('transient', msg);
}
