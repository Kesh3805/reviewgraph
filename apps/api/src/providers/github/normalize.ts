import { z } from 'zod';
import type {
  Ignored,
  IgnoreReason,
  PrRef,
  ProviderActor,
  ProviderEvent,
  PullRequestHeadEventKind,
  RepoRef,
  ReviewCommandName,
} from '../ports';
import { isBotLogin } from './guards';

/**
 * Pure GitHub payload -> provider-neutral event mapping (GH-004). No I/O: permission checks
 * for commands and repository-settings guards run afterwards in `GithubEventNormalizer`.
 * Payloads are validated with zod against only the subset of fields used here.
 */

const user = z.object({ login: z.string().min(1), type: z.string().optional() });
const repository = z.object({
  name: z.string().min(1),
  owner: z.object({ login: z.string().min(1) }),
});
const installation = z.object({ id: z.union([z.number().int(), z.string().min(1)]) });

const pullRequestEvent = z.object({
  action: z.string(),
  installation,
  repository,
  pull_request: z.object({
    number: z.number().int().positive(),
    draft: z.boolean().optional(),
    merged: z.boolean().optional(),
    user,
    head: z.object({ sha: z.string().min(7) }),
    base: z.object({ sha: z.string().min(7), ref: z.string().min(1) }),
  }),
  requested_reviewer: user.optional(),
});

const issueCommentEvent = z.object({
  action: z.string(),
  installation,
  repository,
  issue: z.object({
    number: z.number().int().positive(),
    draft: z.boolean().optional(),
    user,
    pull_request: z.unknown().optional(),
  }),
  comment: z.object({ id: z.union([z.number().int(), z.string().min(1)]), body: z.string(), user }),
});

const HEAD_ACTIONS: readonly string[] = [
  'opened',
  'reopened',
  'synchronize',
  'ready_for_review',
  'review_requested',
];

/** `/review`, `/review full`, `/review cancel`, nothing else on the comment. */
export const REVIEW_COMMAND = /^\/review(\s+(full|cancel))?\s*$/;

/** A command that still needs the commenter's permission checked (I/O, so not done here). */
export interface ReviewCommandCandidate {
  candidate: 'review_command';
  deliveryId: string;
  repo: RepoRef;
  pr: PrRef;
  command: ReviewCommandName;
  commentId: string;
  actor: ProviderActor;
  prAuthor: ProviderActor;
  draft: boolean;
}

export type ParsedGithubEvent = ProviderEvent | ReviewCommandCandidate | Ignored;

export interface NormalizeContext {
  /** The App's bot login (`<slug>[bot]`); `review_requested` only counts for this reviewer. */
  botLogin?: string;
}

const ignored = (reason: IgnoreReason): Ignored => ({ ignored: true, reason });

function actorOf(u: { login: string; type?: string }): ProviderActor {
  return { login: u.login, isBot: u.type === 'Bot' || isBotLogin(u.login) };
}

export function normalizeGithubEvent(
  eventName: string,
  payload: unknown,
  deliveryId: string,
  ctx: NormalizeContext = {},
): ParsedGithubEvent {
  if (eventName === 'pull_request') return normalizePullRequest(payload, deliveryId, ctx);
  if (eventName === 'issue_comment') return normalizeIssueComment(payload, deliveryId);
  return ignored('unsupported_event');
}

function normalizePullRequest(
  payload: unknown,
  deliveryId: string,
  ctx: NormalizeContext,
): ParsedGithubEvent {
  const action = (payload as { action?: unknown } | null)?.action;
  if (typeof action !== 'string') return ignored('malformed');
  if (action !== 'closed' && !HEAD_ACTIONS.includes(action)) return ignored('unsupported_action');

  const parsed = pullRequestEvent.safeParse(payload);
  if (!parsed.success) return ignored('malformed');
  const { installation: inst, repository: repo, pull_request: pr } = parsed.data;
  const installationId = String(inst.id);
  const repoRef: RepoRef = {
    provider: 'github',
    installationId,
    owner: repo.owner.login,
    name: repo.name,
  };
  const prRef: PrRef = { ...repoRef, number: pr.number };

  if (action === 'closed') {
    return {
      type: 'pull_request_closed',
      provider: 'github',
      deliveryId,
      installationId,
      repo: repoRef,
      pr: prRef,
      merged: pr.merged === true,
    };
  }

  let requestedReviewer: string | undefined;
  if (action === 'review_requested') {
    // Only a request addressed to the App's bot starts a review.
    const reviewer = parsed.data.requested_reviewer?.login;
    if (!ctx.botLogin || !reviewer || reviewer.toLowerCase() !== ctx.botLogin.toLowerCase()) {
      return ignored('reviewer_not_app');
    }
    requestedReviewer = reviewer;
  }

  return {
    type: 'pull_request_head',
    kind: action as PullRequestHeadEventKind,
    provider: 'github',
    deliveryId,
    installationId,
    repo: repoRef,
    pr: prRef,
    headSha: pr.head.sha,
    baseSha: pr.base.sha,
    baseRef: pr.base.ref,
    author: actorOf(pr.user),
    draft: pr.draft === true,
    requestedReviewer,
  };
}

function normalizeIssueComment(payload: unknown, deliveryId: string): ParsedGithubEvent {
  const action = (payload as { action?: unknown } | null)?.action;
  if (typeof action !== 'string') return ignored('malformed');
  if (action !== 'created') return ignored('unsupported_action');

  const parsed = issueCommentEvent.safeParse(payload);
  if (!parsed.success) return ignored('malformed');
  const { installation: inst, repository: repo, issue, comment } = parsed.data;
  // Comments on plain issues carry no `pull_request` key.
  if (issue.pull_request === undefined || issue.pull_request === null) {
    return ignored('not_a_pull_request_comment');
  }
  const match = REVIEW_COMMAND.exec(comment.body);
  if (!match) return ignored('not_a_command');

  const actor = actorOf(comment.user);
  // Bots (including this App) never drive commands.
  if (actor.isBot) return ignored('bot_author');

  const installationId = String(inst.id);
  const repoRef: RepoRef = {
    provider: 'github',
    installationId,
    owner: repo.owner.login,
    name: repo.name,
  };
  return {
    candidate: 'review_command',
    deliveryId,
    repo: repoRef,
    pr: { ...repoRef, number: issue.number },
    command: (match[2] as 'full' | 'cancel' | undefined) ?? 'review',
    commentId: String(comment.id),
    actor,
    prAuthor: actorOf(issue.user),
    draft: issue.draft === true,
  };
}

export function isReviewCommandCandidate(e: ParsedGithubEvent): e is ReviewCommandCandidate {
  return 'candidate' in e;
}
