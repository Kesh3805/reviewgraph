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
import { BITBUCKET_API, BitbucketApi, prPath, repoPath } from './bitbucket-auth.service';
import { BitbucketEventNormalizer, lookupPermission } from './bitbucket.normalizer';
import {
  loginOf,
  toProviderPullRequest,
  type BbPullRequest,
  type BbUser,
} from './bitbucket.mapping';
import { verifyBitbucketWebhook } from './bitbucket.webhook';

/** Patches above this size are not fetched per file (the whole diff is one request). */
const MAX_DIFF_BYTES = 8 * 1024 * 1024;

interface BbRepository {
  uuid: string;
  is_private?: boolean;
  mainbranch?: { name: string } | null;
}

interface BbDiffStat {
  status?: string;
  lines_added?: number;
  lines_removed?: number;
  old?: { path: string } | null;
  new?: { path: string } | null;
}

interface BbCommit {
  hash: string;
  message?: string;
  date?: string;
  author?: { user?: BbUser; raw?: string };
  parents?: { hash: string }[];
}

function statusOf(s: string | undefined): ChangedFileStatus {
  if (s === 'added') return 'added';
  if (s === 'removed') return 'removed';
  if (s === 'renamed') return 'renamed';
  return 'modified';
}

/** Hunks of each file in a unified diff, keyed by the file's new path (old path if removed). */
export function splitUnifiedDiff(diff: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const chunk of diff.split(/^diff --git /m).slice(1)) {
    const lines = chunk.split('\n');
    const plus = lines.find((l) => l.startsWith('+++ '));
    const minus = lines.find((l) => l.startsWith('--- '));
    const raw = plus && !plus.startsWith('+++ /dev/null') ? plus : minus;
    const path = raw
      ?.slice(4)
      .replace(/^[ab]\//, '')
      .trim();
    const at = lines.findIndex((l) => l.startsWith('@@'));
    if (path && at >= 0) out.set(path, lines.slice(at).join('\n').trimEnd());
  }
  return out;
}

/**
 * The Bitbucket Cloud `RepositoryProvider` (MP-002). `owner` is the workspace slug and `name`
 * the repository slug; `PrRef.number` is the pull request id. Data Center uses a different API
 * (REST 1.0) and is explicitly deferred (see docs/operations/providers/bitbucket.md).
 */
@Injectable()
export class BitbucketRepositoryProvider implements RepositoryProvider {
  readonly kind = 'bitbucket' as const;

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(BITBUCKET_API) private readonly api: BitbucketApi | null,
    private readonly normalizer: BitbucketEventNormalizer,
  ) {}

  async getRepository(ref: RepoRef): Promise<ProviderRepository> {
    const data = await this.requireApi().get<BbRepository>(repoPath(ref), 'repository lookup');
    return {
      ref,
      providerRepoId: data.uuid,
      defaultBranch: data.mainbranch?.name ?? 'main',
      isPrivate: data.is_private !== false,
      archived: false,
    };
  }

  async getPullRequest(ref: PrRef): Promise<ProviderPullRequest> {
    const pr = await this.requireApi().get<BbPullRequest>(prPath(ref), 'pull request lookup');
    return toProviderPullRequest(ref, pr);
  }

  async *listChangedFiles(ref: PrRef): AsyncIterable<ProviderChangedFile> {
    const api = this.requireApi();
    let patches = new Map<string, string>();
    const diff = await api.text(`${prPath(ref)}/diff`, 'pull request diff');
    if (Buffer.byteLength(diff) <= MAX_DIFF_BYTES) patches = splitUnifiedDiff(diff);
    for await (const s of api.pages<BbDiffStat>(`${prPath(ref)}/diffstat`, 'diffstat listing')) {
      const path = s.new?.path ?? s.old?.path;
      if (!path) continue;
      const patch = patches.get(path);
      yield {
        path,
        ...(s.status === 'renamed' && s.old?.path ? { previousPath: s.old.path } : {}),
        status: statusOf(s.status),
        additions: s.lines_added ?? 0,
        deletions: s.lines_removed ?? 0,
        ...(patch ? { patch } : {}),
      };
    }
  }

  async getCommit(ref: RepoRef, sha: string): Promise<ProviderCommit> {
    const c = await this.requireApi().get<BbCommit>(
      `${repoPath(ref)}/commit/${encodeURIComponent(sha)}`,
      'commit lookup',
    );
    const login = c.author?.user ? loginOf(c.author.user) : undefined;
    return {
      sha: c.hash,
      message: c.message ?? '',
      ...(login ? { authorLogin: login } : {}),
      ...(c.date ? { authoredAt: c.date } : {}),
      parents: (c.parents ?? []).map((p) => p.hash),
    };
  }

  /**
   * The configured access token (scope `repository`, read only is enough for a clone).
   * `ttlSeconds` is advisory: the token's own lifetime decides.
   */
  async issueCloneCredential(ref: RepoRef, ttlSeconds: number): Promise<CloneCredential> {
    const token = await this.requireApi().token.current();
    return { token, expiresAt: new Date(Date.now() + ttlSeconds * 1000), repo: ref };
  }

  verifyWebhook(headers: HeaderBag, rawBody: Buffer): WebhookVerification {
    const secrets = [
      this.config.BITBUCKET_WEBHOOK_SECRET,
      this.config.BITBUCKET_WEBHOOK_SECRET_PREVIOUS,
    ].filter((s): s is string => Boolean(s));
    return verifyBitbucketWebhook(secrets, headers, rawBody);
  }

  normalizeEvent(headers: HeaderBag, body: unknown): Promise<NormalizeResult> {
    const first = (v: string | string[] | undefined): string => (Array.isArray(v) ? v[0] : v) ?? '';
    return this.normalizer.normalize(
      first(headers['x-event-key']),
      body,
      first(headers['x-request-uuid']),
    );
  }

  getActorPermission(ref: RepoRef, login: string): Promise<ActorPermission> {
    return lookupPermission(this.requireApi(), ref, login);
  }

  private requireApi(): BitbucketApi {
    if (!this.api) throw new ProviderError('forbidden', 'Bitbucket integration is disabled');
    return this.api;
  }
}
