import { Inject, Injectable } from '@nestjs/common';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import {
  ProviderError,
  type ActorPermission,
  type ChangedFileStatus,
  type CloneCredential,
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
import { GITLAB_API, GitlabApi, mrPath, projectPath } from './gitlab-auth.service';
import { GitlabEventNormalizer } from './gitlab.normalizer';
import {
  permissionOfAccessLevel,
  toProviderPullRequest,
  type GlMember,
  type GlMergeRequest,
} from './gitlab.mapping';
import { verifyGitlabWebhook } from './gitlab.webhook';

interface GlProject {
  id: number;
  default_branch?: string | null;
  visibility?: string;
  archived?: boolean;
}

interface GlDiff {
  old_path: string;
  new_path: string;
  new_file?: boolean;
  renamed_file?: boolean;
  deleted_file?: boolean;
  diff?: string;
  too_large?: boolean;
  collapsed?: boolean;
}

interface GlCommit {
  id: string;
  message: string;
  author_name?: string;
  authored_date?: string;
  parent_ids?: string[];
}

/** `+`/`-` line counts of a unified diff (hunks only, GitLab sends no file header). */
export function countDiffLines(diff: string): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  for (const line of diff.split('\n')) {
    if (line.startsWith('+')) additions += 1;
    else if (line.startsWith('-')) deletions += 1;
  }
  return { additions, deletions };
}

function statusOf(d: GlDiff): ChangedFileStatus {
  if (d.new_file) return 'added';
  if (d.deleted_file) return 'removed';
  if (d.renamed_file) return 'renamed';
  return 'modified';
}

/**
 * The GitLab `RepositoryProvider` (MP-001): merge request, diff and commit reads over REST v4,
 * webhook verification (`X-Gitlab-Token`) and normalization, and a read-only clone credential.
 * A merge request's `iid` is the neutral `PrRef.number`; `owner/name` is the project's
 * namespace path (subgroups included).
 */
@Injectable()
export class GitlabRepositoryProvider implements RepositoryProvider {
  readonly kind = 'gitlab' as const;

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(GITLAB_API) private readonly api: GitlabApi | null,
    private readonly normalizer: GitlabEventNormalizer,
  ) {}

  async getRepository(ref: RepoRef): Promise<ProviderRepository> {
    const data = await this.requireApi().get<GlProject>(projectPath(ref), 'project lookup');
    return {
      ref,
      providerRepoId: String(data.id),
      defaultBranch: data.default_branch ?? 'main',
      isPrivate: data.visibility !== 'public',
      archived: data.archived === true,
    };
  }

  async getPullRequest(ref: PrRef): Promise<ProviderPullRequest> {
    const mr = await this.requireApi().get<GlMergeRequest>(mrPath(ref), 'merge request lookup');
    return toProviderPullRequest(ref, mr);
  }

  async *listChangedFiles(ref: PrRef): AsyncIterable<ProviderChangedFile> {
    const diffs = this.requireApi().pages<GlDiff>(
      `${mrPath(ref)}/diffs`,
      'merge request diff listing',
    );
    for await (const d of diffs) {
      const patch = d.diff && !d.too_large && !d.collapsed ? d.diff : undefined;
      yield {
        path: d.new_path,
        ...(d.renamed_file ? { previousPath: d.old_path } : {}),
        status: statusOf(d),
        ...countDiffLines(d.diff ?? ''),
        ...(patch ? { patch } : {}),
      };
    }
  }

  async getCommit(ref: RepoRef, sha: string): Promise<ProviderCommit> {
    const c = await this.requireApi().get<GlCommit>(
      `${projectPath(ref)}/repository/commits/${encodeURIComponent(sha)}`,
      'commit lookup',
    );
    return {
      sha: c.id,
      message: c.message,
      ...(c.author_name ? { authorLogin: c.author_name } : {}),
      ...(c.authored_date ? { authoredAt: c.authored_date } : {}),
      parents: c.parent_ids ?? [],
    };
  }

  async listFilesBetween(ref: RepoRef, fromSha: string, toSha: string): Promise<string[]> {
    const data = await this.requireApi().get<{ diffs?: GlDiff[] }>(
      `${projectPath(ref)}/repository/compare`,
      'commit comparison',
      { from: fromSha, to: toSha },
    );
    return (data.diffs ?? []).flatMap((d) =>
      d.renamed_file ? [d.new_path, d.old_path] : [d.new_path],
    );
  }

  /**
   * The configured read-only token (`read_repository`; set GITLAB_CLONE_TOKEN so the API token
   * never reaches a clone). `ttlSeconds` is advisory: the token's own lifetime decides.
   */
  async issueCloneCredential(ref: RepoRef, ttlSeconds: number): Promise<CloneCredential> {
    const api = this.requireApi();
    const token = await api.cloneToken.current();
    return { token, expiresAt: new Date(Date.now() + ttlSeconds * 1000), repo: ref };
  }

  verifyWebhook(headers: HeaderBag): WebhookVerification {
    const secrets = [
      this.config.GITLAB_WEBHOOK_SECRET,
      this.config.GITLAB_WEBHOOK_SECRET_PREVIOUS,
    ].filter((s): s is string => Boolean(s));
    return verifyGitlabWebhook(secrets, headers);
  }

  normalizeEvent(headers: HeaderBag, body: unknown): Promise<NormalizeResult> {
    const first = (v: string | string[] | undefined): string => (Array.isArray(v) ? v[0] : v) ?? '';
    return this.normalizer.normalize(
      first(headers['x-gitlab-event']),
      body,
      first(headers['x-gitlab-event-uuid']),
    );
  }

  async getActorPermission(ref: RepoRef, login: string): Promise<ActorPermission> {
    const members = await this.requireApi().get<GlMember[]>(
      `${projectPath(ref)}/members/all`,
      'member lookup',
      { query: login },
    );
    const member = members.find((m) => m.username.toLowerCase() === login.toLowerCase());
    return member ? permissionOfAccessLevel(member.access_level) : 'none';
  }

  private requireApi(): GitlabApi {
    if (!this.api) throw new ProviderError('forbidden', 'GitLab integration is disabled');
    return this.api;
  }
}
