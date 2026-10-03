import { createHash } from 'node:crypto';
import { Inject, Injectable } from '@nestjs/common';
import { Secret } from '../common/secret';
import { APP_CONFIG, type AppConfig } from '../config/config.module';

/** GitHub could not be reached or answered with a server error. Maps to `github_unavailable`. */
export class GithubUnavailableError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'GithubUnavailableError';
  }
}

/** GitHub rejected the authorization code (expired, reused, wrong verifier). Maps to 400. */
export class OAuthCodeError extends Error {
  constructor() {
    super('authorization code was rejected');
    this.name = 'OAuthCodeError';
  }
}

export interface GithubUser {
  id: number;
  login: string;
  name: string | null;
  email: string | null;
  avatarUrl: string | null;
}

export interface GithubUserInstallation {
  id: number;
  accountLogin: string;
  accountType: 'User' | 'Organization' | string;
}

const REQUEST_TIMEOUT_MS = 10_000;
const PAGE_SIZE = 100;
const MAX_PAGES = 10;

export function pkceChallenge(verifier: string): string {
  return createHash('sha256').update(verifier).digest('base64url');
}

/**
 * The GitHub App user-authorization (OAuth web) flow, with PKCE S256. Plain `fetch`: only the
 * endpoints of the flow are used, and the user token never leaves this class's callers: it is
 * wrapped in `Secret`, used for two read calls during the callback and dropped.
 */
@Injectable()
export class GithubOAuthClient {
  constructor(@Inject(APP_CONFIG) private readonly config: AppConfig) {}

  authorizeUrl(state: string, codeChallenge: string): string {
    const url = new URL('/login/oauth/authorize', this.config.GITHUB_OAUTH_URL);
    url.searchParams.set('client_id', this.config.GITHUB_CLIENT_ID ?? '');
    url.searchParams.set('state', state);
    url.searchParams.set('code_challenge', codeChallenge);
    url.searchParams.set('code_challenge_method', 'S256');
    if (this.config.GITHUB_OAUTH_REDIRECT_URI) {
      url.searchParams.set('redirect_uri', this.config.GITHUB_OAUTH_REDIRECT_URI);
    }
    return url.toString();
  }

  async exchangeCode(code: string, codeVerifier: string): Promise<Secret<string>> {
    const body: Record<string, string> = {
      client_id: this.config.GITHUB_CLIENT_ID ?? '',
      client_secret: this.config.GITHUB_CLIENT_SECRET ?? '',
      code,
      code_verifier: codeVerifier,
    };
    if (this.config.GITHUB_OAUTH_REDIRECT_URI)
      body.redirect_uri = this.config.GITHUB_OAUTH_REDIRECT_URI;
    const res = await this.send(
      new URL('/login/oauth/access_token', this.config.GITHUB_OAUTH_URL),
      {
        method: 'POST',
        headers: { accept: 'application/json', 'content-type': 'application/json' },
        body: JSON.stringify(body),
      },
    );
    const json = (await res.json().catch(() => ({}))) as {
      access_token?: unknown;
      error?: unknown;
    };
    if (typeof json.access_token === 'string' && json.access_token) {
      return new Secret(json.access_token);
    }
    // GitHub answers 200 with `error` for a bad or reused code.
    if (typeof json.error === 'string') throw new OAuthCodeError();
    throw new GithubUnavailableError('token endpoint returned no token');
  }

  async getUser(token: Secret<string>): Promise<GithubUser> {
    const res = await this.api(token, '/user');
    const u = (await res.json()) as {
      id?: unknown;
      login?: unknown;
      name?: unknown;
      email?: unknown;
      avatar_url?: unknown;
    };
    if (typeof u.id !== 'number' || typeof u.login !== 'string') {
      throw new GithubUnavailableError('unexpected /user response');
    }
    return {
      id: u.id,
      login: u.login,
      name: typeof u.name === 'string' ? u.name : null,
      email: typeof u.email === 'string' ? u.email : null,
      avatarUrl: typeof u.avatar_url === 'string' ? u.avatar_url : null,
    };
  }

  /** Installations of this App that the user can access (`GET /user/installations`). */
  async listInstallations(token: Secret<string>): Promise<GithubUserInstallation[]> {
    const out: GithubUserInstallation[] = [];
    for (let page = 1; page <= MAX_PAGES; page++) {
      const res = await this.api(token, `/user/installations?per_page=${PAGE_SIZE}&page=${page}`);
      const json = (await res.json()) as {
        installations?: { id?: unknown; account?: { login?: unknown; type?: unknown } | null }[];
      };
      const items = json.installations ?? [];
      for (const item of items) {
        if (typeof item.id !== 'number' || typeof item.account?.login !== 'string') continue;
        out.push({
          id: item.id,
          accountLogin: item.account.login,
          accountType: typeof item.account.type === 'string' ? item.account.type : 'User',
        });
      }
      if (items.length < PAGE_SIZE) break;
    }
    return out;
  }

  private api(token: Secret<string>, path: string): Promise<Response> {
    return this.send(new URL(path, this.config.GITHUB_API_URL), {
      headers: {
        accept: 'application/vnd.github+json',
        authorization: `Bearer ${token.reveal()}`,
        'user-agent': 'reviewgraph-api',
      },
    });
  }

  private async send(url: URL, init: RequestInit): Promise<Response> {
    let res: Response;
    try {
      res = await fetch(url, { ...init, signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
    } catch {
      throw new GithubUnavailableError('request failed');
    }
    if (!res.ok) throw new GithubUnavailableError(`GitHub answered ${res.status}`);
    return res;
  }
}
