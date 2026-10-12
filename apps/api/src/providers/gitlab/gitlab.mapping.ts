import type { ActorPermission, PrRef, ProviderPullRequest } from '../ports';

/** Stored titles are cut to this many characters; the description is never stored. */
export const MAX_TITLE_LENGTH = 512;

export interface GlUser {
  id?: number;
  username: string;
  bot?: boolean;
}

export interface GlDiffRefs {
  base_sha: string;
  head_sha: string;
  start_sha: string;
}

export interface GlMergeRequest {
  iid: number;
  title: string;
  state: 'opened' | 'closed' | 'merged' | 'locked';
  draft?: boolean;
  work_in_progress?: boolean;
  sha?: string;
  source_branch: string;
  target_branch: string;
  author: GlUser | null;
  labels?: string[];
  updated_at?: string;
  diff_refs?: GlDiffRefs | null;
}

export interface GlMember {
  username: string;
  access_level: number;
}

const BOT_USERNAME = /^(project|group)_\d+_bot/;

export const isGitlabBot = (u: GlUser | null): boolean =>
  u?.bot === true || BOT_USERNAME.test(u?.username ?? '') || (u?.username ?? '').endsWith('[bot]');

/** GitLab project access levels: 10 guest, 20 reporter, 30 developer, 40 maintainer, 50 owner. */
export function permissionOfAccessLevel(level: number): ActorPermission {
  if (level >= 40) return 'admin';
  if (level >= 30) return 'write';
  if (level >= 10) return 'read';
  return 'none';
}

/** Maps a merge request to the provider-neutral model; `diff_refs` carries the real base/head. */
export function toProviderPullRequest(ref: PrRef, mr: GlMergeRequest): ProviderPullRequest {
  const head = mr.diff_refs?.head_sha ?? mr.sha ?? '';
  const login = mr.author?.username ?? 'ghost';
  return {
    ref,
    title: mr.title.slice(0, MAX_TITLE_LENGTH),
    state: mr.state === 'opened' ? 'open' : mr.state === 'merged' ? 'merged' : 'closed',
    draft: mr.draft === true || mr.work_in_progress === true,
    baseSha: mr.diff_refs?.base_sha ?? '',
    headSha: head,
    baseRef: mr.target_branch,
    headRef: mr.source_branch,
    author: { login, isBot: isGitlabBot(mr.author) },
    labels: mr.labels ?? [],
    ...(mr.updated_at ? { updatedAt: mr.updated_at } : {}),
  };
}
