import { Octokit } from '@octokit/rest';
import { retry } from '@octokit/plugin-retry';
import { throttling } from '@octokit/plugin-throttling';
import { incCounter } from '../../common/metrics';

/** Primary rate limits are waited out at most this many times; secondary limits surface at once. */
export const MAX_PRIMARY_RATE_LIMIT_WAITS = 2;

const noop = (): void => undefined;
const ThrottledOctokit = Octokit.plugin(throttling, retry);
export type GithubOctokit = InstanceType<typeof ThrottledOctokit>;

export interface OctokitOptions {
  /** `GITHUB_API_URL`; tests point it at the fake GitHub server. */
  baseUrl: string;
  /** Static credential (an App JWT); installation clients authenticate per request instead. */
  auth?: string;
  fetch?: typeof fetch;
  /** Disable the retry plugin (tests that assert exact request counts). */
  retries?: boolean;
}

/** Octokit with the throttling and retry plugins. Throttle callbacks emit rate-limit metrics. */
export function createOctokit(opts: OctokitOptions): GithubOctokit {
  return new ThrottledOctokit({
    baseUrl: opts.baseUrl,
    auth: opts.auth,
    userAgent: 'reviewgraph-api',
    // Request logging is ours to do (without headers); silence the plugin's console output.
    log: { debug: noop, info: noop, warn: noop, error: noop },
    request: opts.fetch ? { fetch: opts.fetch } : undefined,
    retry: { enabled: opts.retries ?? true },
    throttle: {
      onRateLimit: (_retryAfter, _options, _octokit, retryCount) => {
        incCounter('github_rate_limited_total', { kind: 'primary' });
        return retryCount < MAX_PRIMARY_RATE_LIMIT_WAITS;
      },
      onSecondaryRateLimit: () => {
        incCounter('github_rate_limited_total', { kind: 'secondary' });
        // Back off by surfacing the error; callers see ProviderError{rate_limited}.
        return false;
      },
    },
  });
}
