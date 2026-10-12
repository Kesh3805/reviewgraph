import { SpanStatusCode, trace } from '@opentelemetry/api';
import { incCounter } from '../../common/metrics';
import { TRACER_NAME } from '../../telemetry/tracer.service';
import { ProviderError, type ProviderKind } from '../ports';
import {
  assertResolvesPublic,
  parseBaseUrl,
  type BaseUrlPolicy,
  type HostResolver,
} from './base-url';
import type { TokenSource } from './token-source';

export type FetchLike = (url: string, init: RequestInit) => Promise<Response>;

export interface HttpResult<T> {
  status: number;
  headers: Headers;
  data: T;
}

export interface ProviderRequest {
  method: 'GET' | 'POST' | 'PUT' | 'DELETE';
  /** Path below the base URL, starting with `/`. */
  path: string;
  query?: Record<string, string | number | undefined>;
  body?: unknown;
  /** Human operation name; the only text that ever reaches an error message. */
  operation: string;
  /** Statuses that are an expected answer instead of an error (e.g. 404 on a lookup). */
  accept?: readonly number[];
  /** Body decoding; a unified diff is plain text. Default: JSON. */
  responseType?: 'json' | 'text';
}

export interface ProviderHttpOptions {
  provider: ProviderKind;
  /** Base URL including any API prefix (`https://gitlab.example.com/api/v4`). */
  baseUrl: string;
  tokens: TokenSource;
  /** Header carrying the token (`PRIVATE-TOKEN: x`, `Authorization: Bearer x`). */
  authHeader: (token: string) => Record<string, string>;
  policy?: BaseUrlPolicy;
  fetch?: FetchLike;
  resolveHost?: HostResolver;
  timeoutMs?: number;
  /** Retries for idempotent reads on 5xx, network errors and 429 (exponential backoff). */
  retries?: number;
  sleep?: (ms: number) => Promise<void>;
}

const MAX_BACKOFF_MS = 10_000;
const BASE_BACKOFF_MS = 250;
/** How long a successful DNS check stays valid (limits the DNS rebinding window). */
const DNS_TTL_MS = 5 * 60_000;

/** Seconds (`Retry-After: 30`) or an HTTP date; also GitLab's `RateLimit-Reset` (epoch seconds). */
export function retryAfterMsOf(headers: Headers, now = Date.now()): number | undefined {
  const ra = headers.get('retry-after');
  if (ra) {
    const seconds = Number(ra);
    if (Number.isFinite(seconds) && seconds >= 0) return seconds * 1000;
    const date = Date.parse(ra);
    if (Number.isFinite(date)) return Math.max(0, date - now);
  }
  const reset = Number(headers.get('ratelimit-reset'));
  if (Number.isFinite(reset) && reset > 0) return Math.max(0, reset * 1000 - now);
  return undefined;
}

/**
 * Maps an HTTP status to the provider-neutral `ProviderError`. The message carries only the
 * operation and the status: request headers (credentials) and bodies never enter it.
 */
export function toProviderError(
  status: number,
  headers: Headers,
  operation: string,
): ProviderError {
  const msg = `${operation} failed (status ${status})`;
  if (status === 429)
    return new ProviderError('rate_limited', msg, { retryAfterMs: retryAfterMsOf(headers) });
  if (status === 404) return new ProviderError('not_found', msg);
  if (status === 401 || status === 403) return new ProviderError('forbidden', msg);
  if ([400, 405, 409, 410, 413, 415, 422].includes(status) || (status >= 300 && status < 400)) {
    return new ProviderError('invalid', msg);
  }
  return new ProviderError('transient', msg);
}

/**
 * Minimal JSON-over-HTTP client shared by the GitLab and Bitbucket adapters: SSRF-checked base
 * URL, no redirects followed, a bounded timeout, one credential refresh on 401/403 (then the
 * connection is marked unhealthy), and bounded backoff for idempotent reads. Writes are never
 * retried in-process: a POST whose response was lost may have been applied, and only the
 * publisher's marker lookup can tell.
 */
export class ProviderHttp {
  private readonly base: URL;
  private readonly fetchImpl: FetchLike;
  private readonly retries: number;
  private readonly sleep: (ms: number) => Promise<void>;
  private resolvedAt = 0;

  constructor(private readonly opts: ProviderHttpOptions) {
    this.base = parseBaseUrl(opts.baseUrl, opts.policy);
    this.fetchImpl = opts.fetch ?? ((url, init) => fetch(url, init));
    this.retries = opts.retries ?? 2;
    this.sleep = opts.sleep ?? ((ms) => new Promise((r) => setTimeout(r, ms)));
  }

  get provider(): ProviderKind {
    return this.opts.provider;
  }

  get healthy(): boolean {
    return this.opts.tokens.isHealthy;
  }

  /** Absolute URL of `path` (used to follow pagination links, which must stay on our host). */
  urlOf(path: string, query?: ProviderRequest['query']): string {
    const url = new URL(this.base.href.replace(/\/$/, '') + path);
    for (const [k, v] of Object.entries(query ?? {})) {
      if (v !== undefined) url.searchParams.set(k, String(v));
    }
    return url.href;
  }

  /** True when `link` points at the configured provider host (pagination must not wander off). */
  isSameOrigin(link: string): boolean {
    try {
      return new URL(link, this.base).origin === this.base.origin;
    } catch {
      return false;
    }
  }

  request<T>(req: ProviderRequest): Promise<HttpResult<T>> {
    return this.requestUrl<T>(this.urlOf(req.path, req.query), req);
  }

  /** Same as `request` but for an absolute URL (a `next` link); refused when off-host. */
  async requestUrl<T>(url: string, req: ProviderRequest): Promise<HttpResult<T>> {
    if (!this.isSameOrigin(url)) {
      throw new ProviderError('invalid', `${req.operation} failed (foreign pagination link)`);
    }
    const provider = this.opts.provider;
    return trace
      .getTracer(TRACER_NAME)
      .startActiveSpan(`${provider}_api_request`, { attributes: { provider } }, async (span) => {
        try {
          return await this.send<T>(url, req);
        } catch (err) {
          span.setStatus({ code: SpanStatusCode.ERROR, message: 'provider call failed' });
          throw err;
        } finally {
          span.end();
        }
      });
  }

  private async send<T>(url: string, req: ProviderRequest): Promise<HttpResult<T>> {
    await this.checkHost();
    const idempotent = req.method === 'GET';
    let refreshed = false;
    let attempt = 0;
    for (;;) {
      const token = (await this.opts.tokens.current()).reveal();
      let res: Response;
      try {
        res = await this.fetchImpl(url, {
          method: req.method,
          redirect: 'manual',
          signal: AbortSignal.timeout(this.opts.timeoutMs ?? 15_000),
          headers: {
            accept: 'application/json',
            ...(req.body === undefined ? {} : { 'content-type': 'application/json' }),
            ...this.opts.authHeader(token),
          },
          ...(req.body === undefined ? {} : { body: JSON.stringify(req.body) }),
        });
      } catch {
        incCounter('provider_api_calls_total', {
          provider: this.opts.provider,
          operation: req.operation,
          status: 'network',
        });
        if (idempotent && attempt < this.retries) {
          await this.backoff(attempt++, undefined);
          continue;
        }
        throw new ProviderError('transient', `${req.operation} failed (network error)`);
      }
      incCounter('provider_api_calls_total', {
        provider: this.opts.provider,
        operation: req.operation,
        status: res.status,
      });

      if (res.status >= 200 && res.status < 300) {
        this.opts.tokens.markHealthy();
        return {
          status: res.status,
          headers: res.headers,
          data: await parseBody<T>(res, req.responseType),
        };
      }
      if (req.accept?.includes(res.status)) {
        return { status: res.status, headers: res.headers, data: undefined as T };
      }
      if ((res.status === 401 || res.status === 403) && !refreshed) {
        // Revoke-and-retry once; a second refusal means the credential itself is bad.
        refreshed = true;
        await this.opts.tokens.refresh();
        continue;
      }
      if (res.status === 401 || res.status === 403) {
        this.opts.tokens.markUnhealthy();
        incCounter('provider_connection_unhealthy_total', { provider: this.opts.provider });
      }
      const error = toProviderError(res.status, res.headers, req.operation);
      if (res.status === 429) {
        incCounter('provider_rate_limited_total', { provider: this.opts.provider });
      }
      const retryable = error.kind === 'rate_limited' || error.kind === 'transient';
      if (idempotent && retryable && attempt < this.retries) {
        await this.backoff(attempt++, error.retryAfterMs);
        continue;
      }
      throw error;
    }
  }

  private backoff(attempt: number, retryAfterMs: number | undefined): Promise<void> {
    const wait = retryAfterMs ?? BASE_BACKOFF_MS * 2 ** attempt;
    return this.sleep(Math.min(wait, MAX_BACKOFF_MS));
  }

  private async checkHost(): Promise<void> {
    if (Date.now() - this.resolvedAt < DNS_TTL_MS) return;
    await assertResolvesPublic(this.base, this.opts.policy, this.opts.resolveHost);
    this.resolvedAt = Date.now();
  }
}

async function parseBody<T>(res: Response, type: 'json' | 'text' = 'json'): Promise<T> {
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  if (type === 'text') return text as T;
  if (!text) return undefined as T;
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new ProviderError('transient', 'provider answered with a body that is not JSON');
  }
}
