import { Inject, Injectable, Logger } from '@nestjs/common';
import { z } from 'zod';
import { incCounter } from '../../common/metrics';
import {
  REPOSITORY_SETTINGS,
  type RepositorySettingsPort,
} from '../../repositories/repository-settings.port';
import type { EventNormalizer } from '../../webhooks/webhook.ports';
import { isBotLogin, preReviewGuard } from '../common/guards';
import { REVIEW_COMMAND } from '../common/review-command';
import type {
  ActorPermission,
  Ignored,
  IgnoreReason,
  NormalizeResult,
  PrRef,
  ProviderActor,
  PullRequestClosedEvent,
  PullRequestHeadEvent,
  PullRequestHeadEventKind,
  RepoRef,
  ReviewCommandEvent,
  ReviewCommandName,
} from '../ports';
import { BITBUCKET_API, BitbucketApi, prPath } from './bitbucket-auth.service';
import {
  isBitbucketBot,
  loginOf,
  permissionOfBitbucket,
  toProviderPullRequest,
  type BbPullRequest,
} from './bitbucket.mapping';

const account = z.object({
  nickname: z.string().optional(),
  display_name: z.string().optional(),
  uuid: z.string().optional(),
  type: z.string().optional(),
});
const repository = z.object({ full_name: z.string().min(3) });
const pullrequest = z.object({ id: z.number().int().positive() });

const prHook = z.object({ actor: account.optional(), repository, pullrequest });
const commentHook = z.object({
  actor: account.optional(),
  repository,
  pullrequest,
  comment: z.object({ id: z.number().int(), content: z.object({ raw: z.string() }) }),
});

const ignored = (reason: IgnoreReason): Ignored => ({ ignored: true, reason });

export type BitbucketCandidate =
  | { kind: 'head'; head: PullRequestHeadEventKind; pr: PrRef; deliveryId: string }
  | { kind: 'closed'; pr: PrRef; merged: boolean; deliveryId: string }
  | {
      kind: 'command';
      command: ReviewCommandName;
      pr: PrRef;
      deliveryId: string;
      commentId: string;
      actor: ProviderActor;
    };

/** `workspace/repo`: both halves are slugs. */
function repoRefOf(fullName: string): RepoRef | null {
  const [owner, name, ...rest] = fullName.split('/');
  if (!owner || !name || rest.length > 0) return null;
  // Bitbucket carries no installation id: the workspace identifies the connection.
  return { provider: 'bitbucket', installationId: owner, owner, name };
}

/**
 * Pure payload mapping for `pullrequest:created|updated|fulfilled|rejected` and
 * `pullrequest:comment_created` (`/review` commands). No I/O.
 */
export function parseBitbucketEvent(
  eventKey: string,
  payload: unknown,
  deliveryId: string,
): BitbucketCandidate | Ignored {
  if (eventKey === 'pullrequest:comment_created') {
    const parsed = commentHook.safeParse(payload);
    if (!parsed.success) return ignored('malformed');
    const match = REVIEW_COMMAND.exec(parsed.data.comment.content.raw);
    if (!match) return ignored('not_a_command');
    const repo = repoRefOf(parsed.data.repository.full_name);
    if (!repo) return ignored('malformed');
    const login = loginOf(parsed.data.actor);
    if (isBitbucketBot(parsed.data.actor) || isBotLogin(login)) return ignored('bot_author');
    return {
      kind: 'command',
      command: (match[2] as 'full' | 'cancel' | undefined) ?? 'review',
      pr: { ...repo, number: parsed.data.pullrequest.id },
      deliveryId,
      commentId: String(parsed.data.comment.id),
      actor: { login, isBot: false },
    };
  }
  const head: Record<string, PullRequestHeadEventKind> = {
    'pullrequest:created': 'opened',
    // Fires for new commits and for title edits; an unchanged head is a no-op downstream.
    'pullrequest:updated': 'synchronize',
  };
  const closed: Record<string, boolean> = {
    'pullrequest:fulfilled': true,
    'pullrequest:rejected': false,
  };
  if (!(eventKey in head) && !(eventKey in closed)) return ignored('unsupported_event');
  const parsed = prHook.safeParse(payload);
  if (!parsed.success) return ignored('malformed');
  const repo = repoRefOf(parsed.data.repository.full_name);
  if (!repo) return ignored('malformed');
  const pr: PrRef = { ...repo, number: parsed.data.pullrequest.id };
  const headKind = head[eventKey];
  if (headKind) return { kind: 'head', head: headKind, pr, deliveryId };
  return { kind: 'closed', pr, merged: closed[eventKey] === true, deliveryId };
}

const COMMAND_PERMISSIONS: readonly ActorPermission[] = ['write', 'admin'];

/**
 * Turns a verified Bitbucket delivery into a `ProviderEvent`: payload mapping, then the
 * authoritative pull request (heads never come from the payload), repository-settings guards
 * and, for commands, the commenter's repository permission (fails closed).
 */
@Injectable()
export class BitbucketEventNormalizer implements EventNormalizer {
  private readonly logger = new Logger(BitbucketEventNormalizer.name);

  constructor(
    @Inject(BITBUCKET_API) private readonly api: BitbucketApi | null,
    @Inject(REPOSITORY_SETTINGS) private readonly settings: RepositorySettingsPort,
  ) {}

  async normalize(
    eventName: string,
    payload: unknown,
    deliveryId: string,
  ): Promise<NormalizeResult> {
    const parsed = parseBitbucketEvent(eventName, payload, deliveryId);
    const result = 'ignored' in parsed ? parsed : await this.complete(parsed);
    incCounter('provider_events_total', {
      provider: 'bitbucket',
      kind: 'ignored' in result ? eventName : result.type,
      outcome: 'ignored' in result ? `ignored_${result.reason}` : 'normalized',
    });
    return result;
  }

  private async complete(c: BitbucketCandidate): Promise<NormalizeResult> {
    const repo = repoOf(c.pr);
    if (c.kind === 'closed') {
      const event: PullRequestClosedEvent = {
        type: 'pull_request_closed',
        provider: 'bitbucket',
        deliveryId: c.deliveryId,
        installationId: c.pr.installationId,
        repo,
        pr: c.pr,
        merged: c.merged,
      };
      return event;
    }
    const raw = await this.requireApi().get<BbPullRequest>(prPath(c.pr), 'pull request lookup');
    const pr = toProviderPullRequest(c.pr, raw);
    const settings = await this.settings.getSettings(repo);

    if (c.kind === 'head') {
      const event: PullRequestHeadEvent = {
        type: 'pull_request_head',
        provider: 'bitbucket',
        deliveryId: c.deliveryId,
        installationId: c.pr.installationId,
        repo,
        kind: c.head,
        pr: c.pr,
        headSha: pr.headSha,
        baseSha: pr.baseSha,
        baseRef: pr.baseRef,
        author: pr.author,
        draft: pr.draft,
        ...(pr.updatedAt ? { prUpdatedAt: pr.updatedAt } : {}),
      };
      return preReviewGuard(event, settings) ?? event;
    }

    if (!settings.enabled) return ignored('repository_disabled');
    if (c.command !== 'cancel' && settings.skipBots && pr.author.isBot) {
      return ignored('bot_author');
    }
    let permission: ActorPermission;
    try {
      permission = await this.permissionOf(repo, c.actor.login);
    } catch (err) {
      this.logger.warn(
        `permission lookup failed delivery=${c.deliveryId} (${err instanceof Error ? err.name : 'error'})`,
      );
      return ignored('permission_unknown');
    }
    if (!COMMAND_PERMISSIONS.includes(permission)) return ignored('permission_denied');
    const event: ReviewCommandEvent = {
      type: 'review_command',
      provider: 'bitbucket',
      deliveryId: c.deliveryId,
      installationId: c.pr.installationId,
      repo,
      pr: c.pr,
      command: c.command,
      commentId: c.commentId,
      actor: c.actor,
    };
    return event;
  }

  private async permissionOf(ref: RepoRef, login: string): Promise<ActorPermission> {
    return lookupPermission(this.requireApi(), ref, login);
  }

  private requireApi(): BitbucketApi {
    if (!this.api) throw new Error('Bitbucket integration is disabled');
    return this.api;
  }
}

interface BbPermission {
  permission?: string;
  user?: { nickname?: string; display_name?: string };
}

/** Explicit repository permission of `login` (needs workspace-admin scope; 403 fails closed). */
export async function lookupPermission(
  api: BitbucketApi,
  ref: RepoRef,
  login: string,
): Promise<ActorPermission> {
  const path = `/workspaces/${encodeURIComponent(ref.owner)}/permissions/repositories/${encodeURIComponent(ref.name)}`;
  for await (const p of api.pages<BbPermission>(path, 'permission lookup')) {
    if ((p.user?.nickname ?? p.user?.display_name)?.toLowerCase() === login.toLowerCase()) {
      return permissionOfBitbucket(p.permission);
    }
  }
  return 'none';
}

function repoOf(pr: PrRef): RepoRef {
  return {
    provider: pr.provider,
    installationId: pr.installationId,
    owner: pr.owner,
    name: pr.name,
  };
}
