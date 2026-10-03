import { Inject, Injectable, Logger } from '@nestjs/common';
import { incCounter } from '../../common/metrics';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import {
  REPOSITORY_SETTINGS,
  type RepositorySettingsPort,
} from '../../repositories/repository-settings.port';
import type { EventNormalizer } from '../../webhooks/webhook.ports';
import type { ActorPermission, NormalizeResult, ReviewCommandEvent } from '../ports';
import { isBotLogin, preReviewGuard } from './guards';
import {
  isReviewCommandCandidate,
  normalizeGithubEvent,
  type ReviewCommandCandidate,
} from './normalize';
import { ACTOR_PERMISSION_LOOKUP, type ActorPermissionLookup } from './permissions';

const COMMAND_PERMISSIONS: readonly ActorPermission[] = ['write', 'admin'];

/**
 * Turns a verified GitHub delivery into a `ProviderEvent` (or an ignore reason): the pure
 * payload mapping, then repository-settings guards, then the commenter permission check for
 * `/review` commands. Every uncertainty fails closed (no review). Comment bodies and payload
 * contents are never logged.
 */
@Injectable()
export class GithubEventNormalizer implements EventNormalizer {
  private readonly logger = new Logger(GithubEventNormalizer.name);

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(REPOSITORY_SETTINGS) private readonly settings: RepositorySettingsPort,
    @Inject(ACTOR_PERMISSION_LOOKUP) private readonly permissions: ActorPermissionLookup,
  ) {}

  async normalize(
    eventName: string,
    payload: unknown,
    deliveryId: string,
  ): Promise<NormalizeResult> {
    const botLogin = this.config.GITHUB_APP_SLUG
      ? `${this.config.GITHUB_APP_SLUG}[bot]`
      : undefined;
    const parsed = normalizeGithubEvent(eventName, payload, deliveryId, { botLogin });

    let result: NormalizeResult;
    if ('ignored' in parsed) {
      result = parsed;
    } else if (isReviewCommandCandidate(parsed)) {
      result = await this.authorizeCommand(parsed);
    } else if (parsed.type === 'pull_request_head') {
      const settings = await this.settings.getSettings(parsed.repo);
      result = preReviewGuard(parsed, settings) ?? parsed;
    } else {
      result = parsed;
    }

    const kind =
      'ignored' in result
        ? eventName
        : result.type === 'pull_request_head' || result.type === 'installation'
          ? result.kind
          : result.type;
    incCounter('provider_events_total', {
      kind,
      outcome: 'ignored' in result ? `ignored_${result.reason}` : 'normalized',
    });
    return result;
  }

  private async authorizeCommand(candidate: ReviewCommandCandidate): Promise<NormalizeResult> {
    const settings = await this.settings.getSettings(candidate.repo);
    if (!settings.enabled) return { ignored: true, reason: 'repository_disabled' };
    // `/review cancel` must work on any PR; the bot check protects review starts only.
    if (
      candidate.command !== 'cancel' &&
      settings.skipBots &&
      isBotLogin(candidate.prAuthor.login)
    ) {
      return { ignored: true, reason: 'bot_author' };
    }

    let permission: ActorPermission;
    try {
      permission = await this.permissions.getActorPermission(candidate.repo, candidate.actor.login);
    } catch (err) {
      // Fail closed: an unknown permission never starts a review.
      this.logger.warn(
        `permission lookup failed delivery=${candidate.deliveryId} (${err instanceof Error ? err.name : 'error'})`,
      );
      return { ignored: true, reason: 'permission_unknown' };
    }
    if (!COMMAND_PERMISSIONS.includes(permission)) {
      return { ignored: true, reason: 'permission_denied' };
    }

    const event: ReviewCommandEvent = {
      type: 'review_command',
      provider: 'github',
      deliveryId: candidate.deliveryId,
      installationId: candidate.repo.installationId,
      repo: candidate.repo,
      pr: candidate.pr,
      command: candidate.command,
      commentId: candidate.commentId,
      actor: candidate.actor,
    };
    return event;
  }
}
