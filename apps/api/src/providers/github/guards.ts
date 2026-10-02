import type { RepositorySettings } from '../../repositories/repository-settings.port';
import type { Ignored, PullRequestHeadEvent } from '../ports';

/**
 * Pre-review guards ported from the legacy `pre_submission_guard` / `branch_matches`
 * (`github.rs:305-347`). Self-authored PRs are not a case here: the App's bot is never a PR
 * author.
 */
/** Bot accounts: GitHub `[bot]` logins and the legacy Dependabot name. */
export function isBotLogin(login: string): boolean {
  return login.endsWith('[bot]') || login.toLowerCase() === 'dependabot';
}

/** `release/*` matches by prefix, anything else must match exactly. */
export function branchMatches(branch: string, patterns: readonly string[]): boolean {
  return patterns.some((p) => (p.endsWith('*') ? branch.startsWith(p.slice(0, -1)) : branch === p));
}

/**
 * Returns an ignore decision for a head event, or null to proceed. `explicit` marks a
 * `/review` command, which bypasses the draft guard (the user asked for it).
 */
export function preReviewGuard(
  event: Pick<PullRequestHeadEvent, 'author' | 'draft' | 'baseRef'>,
  settings: RepositorySettings,
  opts: { explicit?: boolean } = {},
): Ignored | null {
  if (!settings.enabled) return { ignored: true, reason: 'repository_disabled' };
  if (settings.skipDrafts && event.draft && !opts.explicit) {
    return { ignored: true, reason: 'draft' };
  }
  if (settings.skipBots && (event.author.isBot || isBotLogin(event.author.login))) {
    return { ignored: true, reason: 'bot_author' };
  }
  if (
    settings.targetBranches.length > 0 &&
    !branchMatches(event.baseRef, settings.targetBranches)
  ) {
    return { ignored: true, reason: 'branch_not_targeted' };
  }
  return null;
}
