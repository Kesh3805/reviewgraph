import type { Secret } from '../../common/secret';

/**
 * Provider-neutral domain model (PRD section 78). Nothing in this file mentions a provider SDK
 * or a provider payload shape; review domain code depends on these types only.
 */

export type ProviderKind = 'github' | 'gitlab' | 'bitbucket';

/** Provider-side identity of a repository. */
export interface RepoRef {
  provider: ProviderKind;
  /** Provider installation that grants access (GitHub App installation id). */
  installationId: string;
  owner: string;
  name: string;
}

export interface PrRef extends RepoRef {
  number: number;
}

export interface ProviderRepository {
  ref: RepoRef;
  /** Provider's numeric/opaque repository id. */
  providerRepoId: string;
  defaultBranch: string;
  isPrivate: boolean;
  archived: boolean;
}

export interface ProviderActor {
  login: string;
  isBot: boolean;
}

export type ProviderPullRequestState = 'open' | 'closed' | 'merged';

export interface ProviderPullRequest {
  ref: PrRef;
  title: string;
  state: ProviderPullRequestState;
  draft: boolean;
  baseSha: string;
  headSha: string;
  baseRef: string;
  headRef: string;
  author: ProviderActor;
  labels: string[];
}

export type ChangedFileStatus =
  'added' | 'modified' | 'removed' | 'renamed' | 'copied' | 'changed' | 'unchanged';

export interface ProviderChangedFile {
  path: string;
  previousPath?: string;
  status: ChangedFileStatus;
  additions: number;
  deletions: number;
  /** Unified-diff hunks for this file; absent for binary or oversized files. */
  patch?: string;
}

export interface ProviderCommit {
  sha: string;
  message: string;
  authorLogin?: string;
  authoredAt?: string;
  parents: string[];
}

/** Read-only, single-repository credential with an expiry. Never persisted. */
export interface CloneCredential {
  token: Secret<string>;
  expiresAt: Date;
  repo: RepoRef;
}

export type ActorPermission = 'admin' | 'write' | 'read' | 'none';

export type WebhookVerification =
  | { valid: true; deliveryId: string; eventName: string }
  | { valid: false; reason: 'missing_signature' | 'bad_signature' | 'missing_headers' };

/**
 * The only review event the system can publish. `APPROVE`, `REQUEST_CHANGES` and anything that
 * could merge are intentionally unrepresentable (INV-011/INV-012).
 */
export type ReviewEvent = 'COMMENT';

export type DiffSide = 'LEFT' | 'RIGHT';

export interface InlineReviewComment {
  path: string;
  /** Last line of the range (new-file numbering for RIGHT, old-file for LEFT). */
  line: number;
  side: DiffSide;
  startLine?: number;
  startSide?: DiffSide;
  body: string;
  /** Stable id of the finding this comment renders (links back to `published_findings`). */
  findingId: string;
}

export interface PublishRequest {
  pr: PrRef;
  /** The head commit the review was produced for. */
  headSha: string;
  event: ReviewEvent;
  /** Review body (the rendered summary). */
  summary: string;
  comments: InlineReviewComment[];
  /** Hidden marker embedded in the review so a retry can find the existing review. */
  marker: string;
}

export interface PublishedComment {
  findingId: string;
  providerCommentId: string;
}

export interface PublishResult {
  providerReviewId: string;
  url?: string;
  comments: PublishedComment[];
}

export interface ExistingReview {
  providerReviewId: string;
  comments: PublishedComment[];
}

export type CheckRunStatus = 'queued' | 'in_progress' | 'completed';
/** Deliberately without `failure`/`action_required`: the check never blocks a merge. */
export type CheckRunConclusion = 'success' | 'neutral' | 'cancelled' | 'skipped';

export interface CheckRunRequest {
  repo: RepoRef;
  headSha: string;
  name: string;
  status: CheckRunStatus;
  conclusion?: CheckRunConclusion;
  title: string;
  summary: string;
  /** Existing check run to update, when known. */
  checkRunId?: string;
}

export interface ResolveResult {
  resolved: string[];
  /** Comment ids that could not be resolved (already deleted, outdated thread). */
  skipped: string[];
}

export type ProviderErrorKind =
  'transient' | 'rate_limited' | 'not_found' | 'forbidden' | 'invalid';

/** Typed provider failure: callers branch on `kind`, never on HTTP status codes. */
export class ProviderError extends Error {
  constructor(
    readonly kind: ProviderErrorKind,
    message: string,
    readonly options: { retryAfterMs?: number; cause?: unknown } = {},
  ) {
    super(message, options.cause === undefined ? undefined : { cause: options.cause });
    this.name = 'ProviderError';
  }

  get retryAfterMs(): number | undefined {
    return this.options.retryAfterMs;
  }

  get retryable(): boolean {
    return this.kind === 'transient' || this.kind === 'rate_limited';
  }
}
