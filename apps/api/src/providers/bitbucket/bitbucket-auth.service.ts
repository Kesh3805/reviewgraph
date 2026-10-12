import type { AppConfig } from '../../config/config.module';
import type { BaseUrlPolicy, HostResolver } from '../common/base-url';
import { ProviderHttp, type FetchLike } from '../common/http-client';
import { TokenSource } from '../common/token-source';
import { ProviderError, type PrRef, type RepoRef } from '../ports';

export const BITBUCKET_API = Symbol('BITBUCKET_API');

/** `owner` is the workspace slug and `name` the repository slug. */
export const repoPath = (ref: RepoRef): string =>
  `/repositories/${encodeURIComponent(ref.owner)}/${encodeURIComponent(ref.name)}`;

export const prPath = (ref: PrRef): string => `${repoPath(ref)}/pullrequests/${ref.number}`;

interface Page<T> {
  values?: T[];
  next?: string;
}

/**
 * Bitbucket Cloud REST 2.0 access for one connection (a repository or workspace access token
 * with the scopes `repository`, `pullrequest` and `pullrequest:write`). Owns `next`-link
 * pagination; everything else goes through the shared `ProviderHttp`.
 */
export class BitbucketApi {
  constructor(
    readonly http: ProviderHttp,
    readonly token: TokenSource,
  ) {}

  async get<T>(path: string, operation: string, query?: Record<string, string | number>) {
    return (await this.http.request<T>({ method: 'GET', path, query, operation })).data;
  }

  async text(path: string, operation: string): Promise<string> {
    const res = await this.http.request<string>({
      method: 'GET',
      path,
      operation,
      responseType: 'text',
    });
    return res.data;
  }

  async post<T>(path: string, operation: string, body: unknown) {
    return (await this.http.request<T>({ method: 'POST', path, body, operation })).data;
  }

  async put<T>(path: string, operation: string, body: unknown, accept?: readonly number[]) {
    return this.http.request<T>({ method: 'PUT', path, body, operation, accept });
  }

  /** Streams every item of a paginated list, following the body's `next` link lazily. */
  async *pages<T>(
    path: string,
    operation: string,
    query: Record<string, string | number> = {},
    maxPages = 50,
  ): AsyncGenerator<T> {
    let next: string | undefined = this.http.urlOf(path, { pagelen: 100, ...query });
    for (let page = 0; next && page < maxPages; page++) {
      const res: { data: Page<T> } = await this.http.requestUrl<Page<T>>(next, {
        method: 'GET',
        path,
        operation,
      });
      for (const item of res.data?.values ?? []) yield item;
      next = res.data?.next;
    }
  }
}

export interface BitbucketApiDeps {
  fetch?: FetchLike;
  resolveHost?: HostResolver;
  sleep?: (ms: number) => Promise<void>;
}

function hostList(raw: string | undefined): string[] {
  return (raw ?? '')
    .split(',')
    .map((h) => h.trim().toLowerCase())
    .filter(Boolean);
}

/** Null when `BITBUCKET_ENABLED=false`. A base URL that fails the SSRF policy fails the boot. */
export function createBitbucketApi(
  config: AppConfig,
  deps: BitbucketApiDeps = {},
): BitbucketApi | null {
  if (!config.BITBUCKET_ENABLED) return null;
  if (!config.BITBUCKET_ACCESS_TOKEN) {
    throw new ProviderError('invalid', 'BITBUCKET_ACCESS_TOKEN is required');
  }
  const policy: BaseUrlPolicy = {
    allowedHosts: hostList(config.BITBUCKET_ALLOWED_HOSTS),
    allowPrivateNetworks: config.BITBUCKET_ALLOW_PRIVATE_NETWORKS,
  };
  const tokens = TokenSource.fixed(config.BITBUCKET_ACCESS_TOKEN);
  const http = new ProviderHttp({
    provider: 'bitbucket',
    baseUrl: config.BITBUCKET_API_URL.replace(/\/+$/, ''),
    tokens,
    authHeader: (token) => ({ authorization: `Bearer ${token}` }),
    policy,
    ...deps,
  });
  return new BitbucketApi(http, tokens);
}
