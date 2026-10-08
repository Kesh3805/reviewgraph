import type { NormalizeResult } from './provider-event';
import type {
  ActorPermission,
  CloneCredential,
  PrRef,
  ProviderChangedFile,
  ProviderCommit,
  ProviderKind,
  ProviderPullRequest,
  ProviderRepository,
  RepoRef,
  WebhookVerification,
} from './types';

export type HeaderBag = Record<string, string | string[] | undefined>;

/**
 * Read side of a code-hosting provider (PRD section 78). Implementations throw
 * `ProviderError` with a `kind`; callers branch on that, never on HTTP status codes.
 */
export interface RepositoryProvider {
  readonly kind: ProviderKind;

  getRepository(ref: RepoRef): Promise<ProviderRepository>;
  /** Base/head sha, author, draft flag, labels and state. */
  getPullRequest(ref: PrRef): Promise<ProviderPullRequest>;
  /** Paginated; implementations stream pages lazily. */
  listChangedFiles(ref: PrRef): AsyncIterable<ProviderChangedFile>;
  getCommit(ref: RepoRef, sha: string): Promise<ProviderCommit>;
  /**
   * Read-only credential scoped to a single repository. `providerRepoId` saves a lookup when the
   * caller already knows it.
   */
  issueCloneCredential(
    ref: RepoRef,
    ttlSeconds: number,
    opts?: { providerRepoId?: string },
  ): Promise<CloneCredential>;
  /** Verifies the delivery signature over the raw body; constant time. */
  verifyWebhook(headers: HeaderBag, rawBody: Buffer): WebhookVerification;
  /**
   * Turns a verified delivery into a provider-neutral event (or an ignore reason). Async because
   * a command needs the commenter's permission, and the lookup fails closed.
   */
  normalizeEvent(headers: HeaderBag, body: unknown): Promise<NormalizeResult>;
  getActorPermission(ref: RepoRef, login: string): Promise<ActorPermission>;
  /**
   * Paths changed between two commits (stale comment resolution, GH-011). Optional: a provider
   * without it makes every absent finding "unknown" rather than "fixed".
   */
  listFilesBetween?(ref: RepoRef, fromSha: string, toSha: string): Promise<string[]>;
}
