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
import { GITLAB_API, GitlabApi, mrPath, projectPath } from './gitlab-auth.service';
import {
  isGitlabBot,
  permissionOfAccessLevel,
  toProviderPullRequest,
  type GlMember,
  type GlMergeRequest,
} from './gitlab.mapping';

const user = z.object({ username: z.string().min(1) });
const project = z.object({
  id: z.number().int(),
  path_with_namespace: z.string().min(3),
});

const mergeRequestHook = z.object({
  user,
  project,
  object_attributes: z.object({
    iid: z.number().int().positive(),
    action: z.string().optional(),
    oldrev: z.string().nullish(),
  }),
  changes: z
    .object({
      draft: z
        .object({ previous: z.boolean().nullish(), current: z.boolean().nullish() })
        .optional(),
    })
    .optional(),
});

const noteHook = z.object({
  user,
  project,
  object_attributes: z.object({
    id: z.number().int(),
    note: z.string(),
    noteable_type: z.string(),
  }),
  merge_request: z.object({ iid: z.number().int().positive() }).optional(),
});

const ignored = (reason: IgnoreReason): Ignored => ({ ignored: true, reason });

/** What the payload alone says; the authoritative merge request is fetched afterwards. */
export type GitlabCandidate =
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

/** `owner/name`: the last path segment is the project, the rest is the namespace. */
function repoRefOf(p: z.infer<typeof project>): RepoRef | null {
  const at = p.path_with_namespace.lastIndexOf('/');
  if (at <= 0 || at === p.path_with_namespace.length - 1) return null;
  return {
    provider: 'gitlab',
    // GitLab has no installation id in a payload; the project id identifies the connection.
    installationId: String(p.id),
    owner: p.path_with_namespace.slice(0, at),
    name: p.path_with_namespace.slice(at + 1),
  };
}

/**
 * Pure payload mapping for `Merge Request Hook` (open, reopen, update with new commits,
 * close, merge) and `Note Hook` (`/review` commands). No I/O.
 */
export function parseGitlabEvent(
  eventName: string,
  payload: unknown,
  deliveryId: string,
): GitlabCandidate | Ignored {
  if (eventName === 'Merge Request Hook') {
    const parsed = mergeRequestHook.safeParse(payload);
    if (!parsed.success) return ignored('malformed');
    const repo = repoRefOf(parsed.data.project);
    if (!repo) return ignored('malformed');
    const attrs = parsed.data.object_attributes;
    const pr: PrRef = { ...repo, number: attrs.iid };
    switch (attrs.action) {
      case 'open':
        return { kind: 'head', head: 'opened', pr, deliveryId };
      case 'reopen':
        return { kind: 'head', head: 'reopened', pr, deliveryId };
      case 'update': {
        // Only an update that moved the source branch (`oldrev` is set) is a new head.
        if (attrs.oldrev) return { kind: 'head', head: 'synchronize', pr, deliveryId };
        const draft = parsed.data.changes?.draft;
        if (draft?.previous === true && draft.current === false) {
          return { kind: 'head', head: 'ready_for_review', pr, deliveryId };
        }
        return ignored('unsupported_action');
      }
      case 'close':
        return { kind: 'closed', pr, merged: false, deliveryId };
      case 'merge':
        return { kind: 'closed', pr, merged: true, deliveryId };
      default:
        return ignored('unsupported_action');
    }
  }
  if (eventName === 'Note Hook') {
    const parsed = noteHook.safeParse(payload);
    if (!parsed.success) return ignored('malformed');
    const attrs = parsed.data.object_attributes;
    if (attrs.noteable_type !== 'MergeRequest' || !parsed.data.merge_request) {
      return ignored('not_a_pull_request_comment');
    }
    const match = REVIEW_COMMAND.exec(attrs.note);
    if (!match) return ignored('not_a_command');
    const repo = repoRefOf(parsed.data.project);
    if (!repo) return ignored('malformed');
    const login = parsed.data.user.username;
    const actor: ProviderActor = { login, isBot: isGitlabBot({ username: login }) };
    // Bots (including this integration's own account) never drive commands.
    if (actor.isBot || isBotLogin(login)) return ignored('bot_author');
    return {
      kind: 'command',
      command: (match[2] as 'full' | 'cancel' | undefined) ?? 'review',
      pr: { ...repo, number: parsed.data.merge_request.iid },
      deliveryId,
      commentId: String(attrs.id),
      actor,
    };
  }
  return ignored('unsupported_event');
}

const COMMAND_PERMISSIONS: readonly ActorPermission[] = ['write', 'admin'];

/**
 * Turns a verified GitLab delivery into a `ProviderEvent`: payload mapping, then the
 * authoritative merge request (so `diff_refs` and the head never come from the payload),
 * repository-settings guards, and the commenter permission check for commands. A failed
 * merge request fetch throws so the delivery is retried; a failed permission lookup fails
 * closed.
 */
@Injectable()
export class GitlabEventNormalizer implements EventNormalizer {
  private readonly logger = new Logger(GitlabEventNormalizer.name);

  constructor(
    @Inject(GITLAB_API) private readonly api: GitlabApi | null,
    @Inject(REPOSITORY_SETTINGS) private readonly settings: RepositorySettingsPort,
  ) {}

  async normalize(
    eventName: string,
    payload: unknown,
    deliveryId: string,
  ): Promise<NormalizeResult> {
    const parsed = parseGitlabEvent(eventName, payload, deliveryId);
    const result = 'ignored' in parsed ? parsed : await this.complete(parsed);
    incCounter('provider_events_total', {
      provider: 'gitlab',
      kind: 'ignored' in result ? eventName : result.type,
      outcome: 'ignored' in result ? `ignored_${result.reason}` : 'normalized',
    });
    return result;
  }

  private async complete(c: GitlabCandidate): Promise<NormalizeResult> {
    if (c.kind === 'closed') {
      const event: PullRequestClosedEvent = {
        type: 'pull_request_closed',
        provider: 'gitlab',
        deliveryId: c.deliveryId,
        installationId: c.pr.installationId,
        repo: repoOf(c.pr),
        pr: c.pr,
        merged: c.merged,
      };
      return event;
    }
    const mr = await this.fetchMergeRequest(c.pr);
    const pr = toProviderPullRequest(c.pr, mr);
    const settings = await this.settings.getSettings(repoOf(c.pr));

    if (c.kind === 'head') {
      const event: PullRequestHeadEvent = {
        type: 'pull_request_head',
        provider: 'gitlab',
        deliveryId: c.deliveryId,
        installationId: c.pr.installationId,
        repo: repoOf(c.pr),
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
    // `/review cancel` must work on any PR; the bot check protects review starts only.
    if (c.command !== 'cancel' && settings.skipBots && pr.author.isBot) {
      return ignored('bot_author');
    }
    let permission: ActorPermission;
    try {
      permission = await this.permissionOf(c.pr, c.actor.login);
    } catch (err) {
      this.logger.warn(
        `permission lookup failed delivery=${c.deliveryId} (${err instanceof Error ? err.name : 'error'})`,
      );
      return ignored('permission_unknown');
    }
    if (!COMMAND_PERMISSIONS.includes(permission)) return ignored('permission_denied');
    const event: ReviewCommandEvent = {
      type: 'review_command',
      provider: 'gitlab',
      deliveryId: c.deliveryId,
      installationId: c.pr.installationId,
      repo: repoOf(c.pr),
      pr: c.pr,
      command: c.command,
      commentId: c.commentId,
      actor: c.actor,
    };
    return event;
  }

  private async fetchMergeRequest(pr: PrRef): Promise<GlMergeRequest> {
    return this.requireApi().get<GlMergeRequest>(mrPath(pr), 'merge request lookup');
  }

  private async permissionOf(ref: RepoRef, login: string): Promise<ActorPermission> {
    const members = await this.requireApi().get<GlMember[]>(
      `${projectPath(ref)}/members/all`,
      'member lookup',
      { query: login },
    );
    const m = members.find((x) => x.username.toLowerCase() === login.toLowerCase());
    return m ? permissionOfAccessLevel(m.access_level) : 'none';
  }

  private requireApi(): GitlabApi {
    if (!this.api) throw new Error('GitLab integration is disabled');
    return this.api;
  }
}

function repoOf(pr: PrRef): RepoRef {
  return {
    provider: pr.provider,
    installationId: pr.installationId,
    owner: pr.owner,
    name: pr.name,
  };
}
