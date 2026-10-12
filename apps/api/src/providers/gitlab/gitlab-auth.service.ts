import type { AppConfig } from '../../config/config.module';
import type { BaseUrlPolicy, HostResolver } from '../common/base-url';
import { ProviderHttp, type FetchLike } from '../common/http-client';
import { TokenSource } from '../common/token-source';
import { ProviderError, type PrRef, type RepoRef } from '../ports';

export const GITLAB_API = Symbol('GITLAB_API');

/** `owner/name` (a namespace path, subgroups included) as the URL-encoded GitLab project id. */
export const projectPath = (ref: RepoRef): string =>
  `/projects/${encodeURIComponent(`${ref.owner}/${ref.name}`)}`;

export const mrPath = (ref: PrRef): string => `${projectPath(ref)}/merge_requests/${ref.number}`;

const LINK_NEXT = /<([^>]+)>;\s*rel="next"/;

/** The `rel="next"` target of a `Link` header, or null on the last page. */
export function nextLink(link: string | null): string | null {
  return LINK_NEXT.exec(link ?? '')?.[1] ?? null;
}

/**
 * GitLab REST v4 access for one connection (a group/project access token or OAuth token,
 * `api` scope only where unavoidable). Owns pagination; everything else goes through the shared
 * `ProviderHttp` (SSRF-checked base URL, refresh-once on 401/403, rate-limit handling).
 */
export class GitlabApi {
  constructor(
    readonly http: ProviderHttp,
    /** Read-only (`read_repository`) token used for clones; falls back to the API token. */
    readonly cloneToken: TokenSource,
    readonly baseUrl: string,
  ) {}

  async get<T>(path: string, operation: string, query?: Record<string, string | number>) {
    return (await this.http.request<T>({ method: 'GET', path, query, operation })).data;
  }

  async post<T>(path: string, operation: string, body: unknown) {
    return (await this.http.request<T>({ method: 'POST', path, body, operation })).data;
  }

  async put<T>(path: string, operation: string, query?: Record<string, string | number>) {
    return (await this.http.request<T>({ method: 'PUT', path, query, operation })).data;
  }

  /** Streams every item of a paginated list, following `Link: rel="next"` lazily. */
  async *pages<T>(
    path: string,
    operation: string,
    query: Record<string, string | number> = {},
    maxPages = 50,
  ): AsyncGenerator<T> {
    let next: string | null = this.http.urlOf(path, { per_page: 100, ...query });
    for (let page = 0; next && page < maxPages; page++) {
      const res: { data: T[]; headers: Headers } = await this.http.requestUrl<T[]>(next, {
        method: 'GET',
        path,
        operation,
      });
      for (const item of res.data ?? []) yield item;
      next = nextLink(res.headers.get('link'));
    }
  }
}

export interface GitlabApiDeps {
  fetch?: FetchLike;
  resolveHost?: HostResolver;
  sleep?: (ms: number) => Promise<void>;
}

/** Splits a comma-separated host list from configuration. */
function hostList(raw: string | undefined): string[] {
  return (raw ?? '')
    .split(',')
    .map((h) => h.trim().toLowerCase())
    .filter(Boolean);
}

/** Null when `GITLAB_ENABLED=false`. A base URL that fails the SSRF policy fails the boot. */
export function createGitlabApi(config: AppConfig, deps: GitlabApiDeps = {}): GitlabApi | null {
  if (!config.GITLAB_ENABLED) return null;
  if (!config.GITLAB_TOKEN) throw new ProviderError('invalid', 'GITLAB_TOKEN is required');
  const policy: BaseUrlPolicy = {
    allowedHosts: hostList(config.GITLAB_ALLOWED_HOSTS),
    allowPrivateNetworks: config.GITLAB_ALLOW_PRIVATE_NETWORKS,
  };
  const root = config.GITLAB_BASE_URL.replace(/\/+$/, '');
  const tokens = TokenSource.fixed(config.GITLAB_TOKEN);
  const http = new ProviderHttp({
    provider: 'gitlab',
    baseUrl: `${root}/api/v4`,
    tokens,
    authHeader: (token) => ({ 'private-token': token }),
    policy,
    ...deps,
  });
  const cloneToken = config.GITLAB_CLONE_TOKEN
    ? TokenSource.fixed(config.GITLAB_CLONE_TOKEN)
    : tokens;
  return new GitlabApi(http, cloneToken, root);
}
