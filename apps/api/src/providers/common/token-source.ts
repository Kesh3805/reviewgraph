import { Secret } from '../../common/secret';

export interface LoadedToken {
  token: string;
  /** Absent for a long-lived access token. */
  expiresAt?: Date;
}

/** Early-refresh margin for expiring tokens. */
const REFRESH_MARGIN_MS = 60_000;

/**
 * Token for a provider connection. `current()` serves the cached token; `refresh()` throws the
 * cached one away and loads again. Concurrent callers share ONE load (single flight), so a
 * burst of 401s mints one new token, not one per request. Tokens live in memory only: never in
 * Postgres, never in a log line (`Secret` redacts itself).
 */
export class TokenSource {
  private cached?: { secret: Secret<string>; expiresAt?: Date };
  private inflight?: Promise<Secret<string>>;
  private healthy = true;

  constructor(
    private readonly load: () => Promise<LoadedToken>,
    private readonly now: () => number = Date.now,
  ) {}

  /** A source for a static access token (project/group/workspace token from configuration). */
  static fixed(token: string): TokenSource {
    return new TokenSource(() => Promise.resolve({ token }));
  }

  async current(): Promise<Secret<string>> {
    const hit = this.cached;
    if (hit && (!hit.expiresAt || hit.expiresAt.getTime() - REFRESH_MARGIN_MS > this.now())) {
      return hit.secret;
    }
    return this.refresh();
  }

  refresh(): Promise<Secret<string>> {
    this.inflight ??= (async () => {
      try {
        const loaded = await this.load();
        const secret = new Secret(loaded.token);
        this.cached = { secret, ...(loaded.expiresAt ? { expiresAt: loaded.expiresAt } : {}) };
        return secret;
      } finally {
        this.inflight = undefined;
      }
    })();
    return this.inflight;
  }

  /** False once a refreshed credential was still refused: the connection needs attention. */
  get isHealthy(): boolean {
    return this.healthy;
  }

  markUnhealthy(): void {
    this.healthy = false;
  }

  markHealthy(): void {
    this.healthy = true;
  }
}
