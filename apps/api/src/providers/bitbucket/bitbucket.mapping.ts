import type { ActorPermission, PrRef, ProviderPullRequest } from '../ports';

/** Stored titles are cut to this many characters; the description is never stored. */
export const MAX_TITLE_LENGTH = 512;

export interface BbUser {
  nickname?: string;
  display_name?: string;
  uuid?: string;
  type?: string;
}

export interface BbPullRequest {
  id: number;
  title: string;
  state: 'OPEN' | 'MERGED' | 'DECLINED' | 'SUPERSEDED';
  draft?: boolean;
  author?: BbUser | null;
  source: { branch: { name: string }; commit?: { hash: string } | null };
  destination: { branch: { name: string }; commit?: { hash: string } | null };
  updated_on?: string;
}

/** The login of a Bitbucket account (nickname, falling back to the display name). */
export const loginOf = (u: BbUser | null | undefined): string =>
  u?.nickname ?? u?.display_name ?? 'ghost';

/** Bitbucket repository/workspace access tokens act as `team`-type or `*_bot` accounts. */
export const isBitbucketBot = (u: BbUser | null | undefined): boolean =>
  u?.type === 'team' || /(^|[-_])bot$/i.test(loginOf(u));

/** Bitbucket repository permissions: `admin`, `write`, `read`. */
export function permissionOfBitbucket(permission: string | undefined): ActorPermission {
  return permission === 'admin' || permission === 'write' || permission === 'read'
    ? permission
    : 'none';
}

/**
 * Maps a pull request to the provider-neutral model. `base` is the destination branch tip
 * (Bitbucket reports no merge base on the pull request itself).
 */
export function toProviderPullRequest(ref: PrRef, pr: BbPullRequest): ProviderPullRequest {
  return {
    ref,
    title: pr.title.slice(0, MAX_TITLE_LENGTH),
    state: pr.state === 'OPEN' ? 'open' : pr.state === 'MERGED' ? 'merged' : 'closed',
    draft: pr.draft === true,
    baseSha: pr.destination.commit?.hash ?? '',
    headSha: pr.source.commit?.hash ?? '',
    baseRef: pr.destination.branch.name,
    headRef: pr.source.branch.name,
    author: { login: loginOf(pr.author), isBot: isBitbucketBot(pr.author) },
    labels: [],
    ...(pr.updated_on ? { updatedAt: pr.updated_on } : {}),
  };
}
