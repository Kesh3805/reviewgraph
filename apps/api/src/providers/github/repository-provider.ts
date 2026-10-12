import { Inject, Injectable } from '@nestjs/common';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import { verifyWebhookSignature } from '../../webhooks/signature';
import {
  ProviderError,
  type ActorPermission,
  type ChangedFileStatus,
  type CloneCredential,
  type HistoryPage,
  type HistoryPullRequest,
  type HistoryReviewComment,
  type HeaderBag,
  type NormalizeResult,
  type PrRef,
  type ProviderChangedFile,
  type ProviderCommit,
  type ProviderPullRequest,
  type ProviderRepository,
  type RepoRef,
  type RepositoryProvider,
  type WebhookVerification,
} from '../ports';
import type { GithubAppAuth } from './app-auth.service';
import { githubCall } from './api-call';
import { GithubEventNormalizer } from './event-normalizer.service';
import { GITHUB_APP_AUTH } from './github.tokens';
import type { GithubOctokit } from './octokit.factory';
import { ACTOR_PERMISSION_LOOKUP, type ActorPermissionLookup } from './permissions';

export const FILES_PER_PAGE = 100;
/** Comment fetches in flight while building one history page (HIST-001). */
export const HISTORY_COMMENT_CONCURRENCY = 4;
const HISTORY_COMMENTS_PER_PULL = 100;
/** Stored titles are cut to this many characters; the body is never stored. */
export const MAX_TITLE_LENGTH = 512;

interface GhUser {
  login: string;
  type?: string;
}

interface GhRepository {
  id: number;
  name: string;
  owner: { login: string };
  default_branch: string;
  private: boolean;
  archived?: boolean;
}

interface GhPullRequest {
  number: number;
  title: string;
  state: 'open' | 'closed';
  merged?: boolean;
  merged_at?: string | null;
  draft?: boolean;
  user: GhUser | null;
  head: { sha: string; ref: string };
  base: { sha: string; ref: string };
  labels?: { name?: string }[];
  updated_at?: string;
}

interface GhHistoryPull extends GhPullRequest {
  id: number;
  created_at: string;
  closed_at: string | null;
  merge_commit_sha?: string | null;
}

interface GhReviewComment {
  id: number;
  in_reply_to_id?: number;
  path?: string;
  line?: number | null;
  original_line?: number | null;
  commit_id?: string;
  user: GhUser | null;
  body: string;
  created_at: string;
}

const isBotUser = (user: GhUser | null): boolean =>
  user?.type === 'Bot' || (user?.login ?? '').endsWith('[bot]');

interface GhFile {
  filename: string;
  previous_filename?: string;
  status: ChangedFileStatus;
  additions: number;
  deletions: number;
  patch?: string;
}

interface GhCommit {
  sha: string;
  commit: { message: string; author?: { date?: string } | null };
  author?: { login?: string } | null;
  parents: { sha: string }[];
}

/** Maps a GitHub pull request to the provider-neutral model (title truncated, no body). */
export function toProviderPullRequest(ref: PrRef, pr: GhPullRequest): ProviderPullRequest {
  const login = pr.user?.login ?? 'ghost';
  return {
    ref,
    title: pr.title.slice(0, MAX_TITLE_LENGTH),
    state: pr.state === 'open' ? 'open' : pr.merged || pr.merged_at ? 'merged' : 'closed',
    draft: pr.draft === true,
    baseSha: pr.base.sha,
    headSha: pr.head.sha,
    baseRef: pr.base.ref,
    headRef: pr.head.ref,
    author: { login, isBot: pr.user?.type === 'Bot' || login.endsWith('[bot]') },
    labels: (pr.labels ?? []).map((l) => l.name ?? '').filter(Boolean),
    ...(pr.updated_at ? { updatedAt: pr.updated_at } : {}),
  };
}

/**
 * The GitHub `RepositoryProvider` (GH-005, GH-006): authoritative repository, pull request,
 * file and commit reads through the installation client (Octokit retries transient failures),
 * webhook verification and normalization, and read-only single-repository clone credentials.
 */
@Injectable()
export class GithubRepositoryProvider implements RepositoryProvider {
  readonly kind = 'github' as const;

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(GITHUB_APP_AUTH) private readonly auth: GithubAppAuth | null,
    private readonly normalizer: GithubEventNormalizer,
    @Inject(ACTOR_PERMISSION_LOOKUP) private readonly permissions: ActorPermissionLookup,
  ) {}

  async getRepository(ref: RepoRef): Promise<ProviderRepository> {
    const octokit = await this.octokit(ref);
    const { data } = await githubCall<GhRepository>(
      'GET /repos/{owner}/{repo}',
      'repository lookup',
      async () =>
        (await octokit.request('GET /repos/{owner}/{repo}', {
          owner: ref.owner,
          repo: ref.name,
        })) as { status: number; data: GhRepository },
    );
    return {
      ref,
      providerRepoId: String(data.id),
      defaultBranch: data.default_branch,
      isPrivate: data.private,
      archived: data.archived === true,
    };
  }

  async getPullRequest(ref: PrRef): Promise<ProviderPullRequest> {
    const octokit = await this.octokit(ref);
    const { data } = await githubCall<GhPullRequest>(
      'GET /repos/{owner}/{repo}/pulls/{pull_number}',
      'pull request lookup',
      async () =>
        (await octokit.request('GET /repos/{owner}/{repo}/pulls/{pull_number}', {
          owner: ref.owner,
          repo: ref.name,
          pull_number: ref.number,
        })) as { status: number; data: GhPullRequest },
      { span: 'github_get_pull_request', attributes: { 'github.pr': ref.number } },
    );
    return toProviderPullRequest(ref, data);
  }

  /** Pages lazily (100 per page) until a short page; GitHub itself stops at 3,000 files. */
  async *listChangedFiles(ref: PrRef): AsyncIterable<ProviderChangedFile> {
    const octokit = await this.octokit(ref);
    for (let page = 1; ; page++) {
      const { data } = await githubCall<GhFile[]>(
        'GET /repos/{owner}/{repo}/pulls/{pull_number}/files',
        'pull request file listing',
        async () =>
          (await octokit.request('GET /repos/{owner}/{repo}/pulls/{pull_number}/files', {
            owner: ref.owner,
            repo: ref.name,
            pull_number: ref.number,
            per_page: FILES_PER_PAGE,
            page,
          })) as { status: number; data: GhFile[] },
        { span: 'github_list_files', attributes: { 'github.pr': ref.number, page } },
      );
      for (const f of data) {
        yield {
          path: f.filename,
          ...(f.previous_filename ? { previousPath: f.previous_filename } : {}),
          status: f.status,
          additions: f.additions,
          deletions: f.deletions,
          ...(f.patch !== undefined ? { patch: f.patch } : {}),
        };
      }
      if (data.length < FILES_PER_PAGE) return;
    }
  }

  async getCommit(ref: RepoRef, sha: string): Promise<ProviderCommit> {
    const octokit = await this.octokit(ref);
    const { data } = await githubCall<GhCommit>(
      'GET /repos/{owner}/{repo}/commits/{ref}',
      'commit lookup',
      async () =>
        (await octokit.request('GET /repos/{owner}/{repo}/commits/{ref}', {
          owner: ref.owner,
          repo: ref.name,
          ref: sha,
        })) as { status: number; data: GhCommit },
    );
    return {
      sha: data.sha,
      message: data.commit.message,
      ...(data.author?.login ? { authorLogin: data.author.login } : {}),
      ...(data.commit.author?.date ? { authoredAt: data.commit.author.date } : {}),
      parents: data.parents.map((p) => p.sha),
    };
  }

  /**
   * `GET /repos/{o}/{r}/pulls?state=closed&sort=updated`, then the review comments of each pull
   * request (at most four in flight). REST does not expose thread resolution, so `resolved` is
   * false here; the GraphQL thread sync (HIST-002) fills it in.
   */
  async listHistoryPullRequests(
    ref: RepoRef,
    opts: { cursor?: string; perPage: number },
  ): Promise<HistoryPage> {
    const octokit = await this.octokit(ref);
    const page = Math.max(1, Number.parseInt(opts.cursor ?? '1', 10) || 1);
    const { data } = await githubCall<GhHistoryPull[]>(
      'GET /repos/{owner}/{repo}/pulls',
      'history pull request listing',
      async () =>
        (await octokit.request('GET /repos/{owner}/{repo}/pulls', {
          owner: ref.owner,
          repo: ref.name,
          state: 'closed',
          sort: 'updated',
          direction: 'desc',
          per_page: opts.perPage,
          page,
        })) as { status: number; data: GhHistoryPull[] },
      { span: 'github_list_history_pulls', attributes: { page } },
    );
    const pullRequests: HistoryPullRequest[] = [];
    for (let i = 0; i < data.length; i += HISTORY_COMMENT_CONCURRENCY) {
      const batch = data.slice(i, i + HISTORY_COMMENT_CONCURRENCY);
      pullRequests.push(
        ...(await Promise.all(batch.map((pr) => this.toHistoryPull(octokit, ref, pr)))),
      );
    }
    return {
      pullRequests,
      ...(data.length === opts.perPage ? { nextCursor: String(page + 1) } : {}),
    };
  }

  private async toHistoryPull(
    octokit: GithubOctokit,
    ref: RepoRef,
    pr: GhHistoryPull,
  ): Promise<HistoryPullRequest> {
    const { data } = await githubCall<GhReviewComment[]>(
      'GET /repos/{owner}/{repo}/pulls/{pull_number}/comments',
      'history review comments',
      async () =>
        (await octokit.request('GET /repos/{owner}/{repo}/pulls/{pull_number}/comments', {
          owner: ref.owner,
          repo: ref.name,
          pull_number: pr.number,
          per_page: HISTORY_COMMENTS_PER_PULL,
        })) as { status: number; data: GhReviewComment[] },
      { span: 'github_list_history_comments', attributes: { 'github.pr': pr.number } },
    );
    const comments: HistoryReviewComment[] = data.map((c) => {
      const line = c.line ?? c.original_line;
      return {
        providerCommentId: String(c.id),
        threadId: String(c.in_reply_to_id ?? c.id),
        ...(c.path ? { path: c.path } : {}),
        ...(line ? { line } : {}),
        ...(c.commit_id ? { commitSha: c.commit_id } : {}),
        authorLogin: c.user?.login ?? 'ghost',
        authorIsBot: isBotUser(c.user),
        body: c.body,
        resolved: false,
        resolvedByIsBot: false,
        createdAt: c.created_at,
      };
    });
    return {
      providerPrId: String(pr.id),
      number: pr.number,
      title: pr.title.slice(0, MAX_TITLE_LENGTH),
      authorLogin: pr.user?.login ?? 'ghost',
      merged: pr.merged === true || Boolean(pr.merged_at),
      ...(pr.merge_commit_sha ? { mergeCommit: pr.merge_commit_sha } : {}),
      openedAt: pr.created_at,
      ...(pr.closed_at ? { closedAt: pr.closed_at } : {}),
      updatedAt: pr.updated_at ?? pr.closed_at ?? pr.created_at,
      // The list endpoint omits size statistics; the single-PR read is not worth one call per PR.
      additions: 0,
      deletions: 0,
      changedFiles: 0,
      comments,
    };
  }

  /** `GET /repos/{o}/{r}/compare/{from}...{to}`: the paths changed between two commits. */
  async listFilesBetween(ref: RepoRef, fromSha: string, toSha: string): Promise<string[]> {
    const octokit = await this.octokit(ref);
    const { data } = await githubCall<{ files?: GhFile[] }>(
      'GET /repos/{owner}/{repo}/compare/{basehead}',
      'commit comparison',
      async () =>
        (await octokit.request('GET /repos/{owner}/{repo}/compare/{basehead}', {
          owner: ref.owner,
          repo: ref.name,
          basehead: `${fromSha}...${toSha}`,
        })) as { status: number; data: { files?: GhFile[] } },
    );
    return (data.files ?? []).flatMap((f) =>
      f.previous_filename ? [f.filename, f.previous_filename] : [f.filename],
    );
  }

  /**
   * A token limited to this one repository with `contents: read` (GH-006). `ttlSeconds` is
   * advisory: GitHub decides the lifetime (one hour), and the cache refreshes ten minutes early.
   */
  async issueCloneCredential(
    ref: RepoRef,
    _ttlSeconds: number,
    opts: { providerRepoId?: string } = {},
  ): Promise<CloneCredential> {
    const auth = this.requireAuth();
    const providerRepoId = opts.providerRepoId ?? (await this.getRepository(ref)).providerRepoId;
    const token = await auth.getInstallationToken(ref.installationId, {
      repositoryIds: [Number(providerRepoId)],
      permissions: { contents: 'read' },
    });
    return { token: token.token, expiresAt: token.expiresAt, repo: ref };
  }

  verifyWebhook(headers: HeaderBag, rawBody: Buffer): WebhookVerification {
    const header = (name: string): string | undefined => {
      const v = headers[name];
      return Array.isArray(v) ? v[0] : v;
    };
    const signature = header('x-hub-signature-256');
    const deliveryId = header('x-github-delivery');
    const eventName = header('x-github-event');
    const secrets = [
      this.config.GITHUB_WEBHOOK_SECRET,
      this.config.GITHUB_WEBHOOK_SECRET_PREVIOUS,
    ].filter((s): s is string => Boolean(s));
    const verdict = verifyWebhookSignature(secrets, rawBody, signature);
    if (verdict === 'missing') return { valid: false, reason: 'missing_signature' };
    if (verdict !== 'valid') return { valid: false, reason: 'bad_signature' };
    if (!deliveryId || !eventName) return { valid: false, reason: 'missing_headers' };
    return { valid: true, deliveryId, eventName };
  }

  normalizeEvent(headers: HeaderBag, body: unknown): Promise<NormalizeResult> {
    const first = (v: string | string[] | undefined): string => (Array.isArray(v) ? v[0] : v) ?? '';
    return this.normalizer.normalize(
      first(headers['x-github-event']),
      body,
      first(headers['x-github-delivery']),
    );
  }

  getActorPermission(ref: RepoRef, login: string): Promise<ActorPermission> {
    return this.permissions.getActorPermission(ref, login);
  }

  private requireAuth(): GithubAppAuth {
    if (!this.auth) throw new ProviderError('forbidden', 'GitHub integration is disabled');
    return this.auth;
  }

  private octokit(ref: RepoRef): Promise<GithubOctokit> {
    return this.requireAuth().getOctokit(ref.installationId);
  }
}
