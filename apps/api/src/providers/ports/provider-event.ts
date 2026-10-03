import type { ProviderActor, ProviderKind, PrRef, RepoRef } from './types';

/**
 * Provider-neutral events produced by webhook normalization (GH-004). The orchestrator
 * (reviews module) consumes these and never sees a provider payload.
 */

interface ProviderEventBase {
  provider: ProviderKind;
  /** Provider delivery id, used for idempotency. */
  deliveryId: string;
  installationId: string;
  repo: RepoRef;
}

export type PullRequestHeadEventKind =
  'opened' | 'reopened' | 'synchronize' | 'ready_for_review' | 'review_requested';

/** A pull request head became (or may have become) reviewable. */
export interface PullRequestHeadEvent extends ProviderEventBase {
  type: 'pull_request_head';
  kind: PullRequestHeadEventKind;
  pr: PrRef;
  headSha: string;
  baseSha: string;
  baseRef: string;
  author: ProviderActor;
  draft: boolean;
  requestedReviewer?: string;
}

/** The pull request was closed or merged: active runs are cancelled. */
export interface PullRequestClosedEvent extends ProviderEventBase {
  type: 'pull_request_closed';
  pr: PrRef;
  merged: boolean;
}

export type ReviewCommandName = 'review' | 'full' | 'cancel';

/**
 * An explicit `/review` command. Only emitted when the commenter holds write or admin permission
 * (the normalizer fails closed otherwise).
 */
export interface ReviewCommandEvent extends ProviderEventBase {
  type: 'review_command';
  command: ReviewCommandName;
  pr: PrRef;
  /** Provider id of the comment, used for the acknowledgement reaction. */
  commentId: string;
  actor: ProviderActor;
}

export type ProviderEvent = PullRequestHeadEvent | PullRequestClosedEvent | ReviewCommandEvent;

export type InstallationEventKind =
  | 'created'
  | 'deleted'
  | 'suspend'
  | 'unsuspend'
  | 'new_permissions_accepted'
  | 'repositories_added'
  | 'repositories_removed';

/** A repository named by an installation event (the payload carries no more than this). */
export interface InstallationRepository {
  providerRepoId: string;
  fullName: string;
  isPrivate: boolean;
}

/**
 * Installation lifecycle (GH-013): install, uninstall, suspend, permission change and repository
 * access changes. Handled by the installation service inside the delivery transaction; it is
 * never handed to the review orchestrator, hence not part of `ProviderEvent`.
 */
export interface InstallationEvent {
  type: 'installation';
  provider: ProviderKind;
  deliveryId: string;
  installationId: string;
  kind: InstallationEventKind;
  account: { login: string; kind: 'organization' | 'user' };
  /** Permissions granted to the App on this installation, as reported by the provider. */
  permissions: Record<string, string>;
  /** Repositories gained (`created`, `repositories_added`). */
  added: InstallationRepository[];
  /** Repositories lost (`repositories_removed`). */
  removed: InstallationRepository[];
}

export type IgnoreReason =
  | 'unsupported_event'
  | 'unsupported_action'
  | 'malformed'
  | 'draft'
  | 'bot_author'
  | 'branch_not_targeted'
  | 'repository_disabled'
  | 'not_a_pull_request_comment'
  | 'not_a_command'
  | 'permission_denied'
  | 'permission_unknown'
  | 'reviewer_not_app'
  | 'unknown_installation';

/** The event is valid but needs no review work. Recorded as `webhook_deliveries.outcome`. */
export interface Ignored {
  ignored: true;
  reason: IgnoreReason;
}

export type NormalizeResult = ProviderEvent | InstallationEvent | Ignored;

export function isIgnored(result: NormalizeResult): result is Ignored {
  return 'ignored' in result;
}
