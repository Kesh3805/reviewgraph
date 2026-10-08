import { Inject, Injectable, Logger } from '@nestjs/common';
import { incCounter } from '../common/metrics';
import { LOCK_NOT_AVAILABLE } from '../db/errors';
import { DbService } from '../db/db.module';
import {
  PROVIDER_RESOLVER,
  ProviderError,
  type ProviderEvent,
  type ProviderPullRequest,
  type ProviderResolver,
  type PullRequestClosedEvent,
  type PullRequestHeadEvent,
  type ReviewCommandEvent,
} from '../providers/ports';
import { RepositorySyncService, type SyncedRepository } from '../repositories/sync.service';
import type { ProviderEventSink } from '../webhooks/webhook.ports';
import { PullRequestSyncService, providerUpdatedAt } from './pull-request-sync.service';
import {
  SupersessionService,
  type ReviewDepth,
  type ReviewTrigger,
  type StartReviewResult,
} from './supersession.service';

/** Attempts for a head update that keeps hitting the PR lock (a publish is in flight). */
export const LOCK_RETRY_ATTEMPTS = 4;
const LOCK_RETRY_BASE_MS = 1000;

/** Synthetic delivery ids of the polling reconciler (GH-012). */
export const isPollDelivery = (deliveryId: string): boolean => deliveryId.startsWith('poll:');

function isLockTimeout(err: unknown): boolean {
  return (err as { code?: unknown } | null)?.code === LOCK_NOT_AVAILABLE;
}

export type DispatchOutcome =
  | StartReviewResult['outcome']
  | 'cancelled'
  | 'ignored_unknown_installation'
  | 'ignored_access_lost'
  | 'ignored_repository_disabled'
  | 'ignored_unknown_pull_request';

/**
 * The PR review orchestrator behind the webhook sink and the reconciler: provider events become
 * repository and pull request syncs (GH-005) followed by supersession (SUP-001) or cancellation.
 * Webhook payloads can be stale, so the head is always taken from an authoritative read.
 */
@Injectable()
export class ReviewOrchestrator implements ProviderEventSink {
  private readonly logger = new Logger(ReviewOrchestrator.name);

  constructor(
    private readonly dbs: DbService,
    private readonly repositories: RepositorySyncService,
    private readonly pullRequests: PullRequestSyncService,
    private readonly supersession: SupersessionService,
    @Inject(PROVIDER_RESOLVER) private readonly providers: ProviderResolver,
  ) {}

  async dispatch(event: ProviderEvent): Promise<void> {
    await this.handle(event);
  }

  async handle(event: ProviderEvent): Promise<DispatchOutcome> {
    let outcome: DispatchOutcome;
    switch (event.type) {
      case 'pull_request_head':
        outcome = await this.onHead(
          event,
          isPollDelivery(event.deliveryId) ? 'reconciler' : 'webhook',
        );
        break;
      case 'pull_request_closed':
        outcome = await this.onClosed(event);
        break;
      case 'review_command':
        outcome = await this.onCommand(event);
        break;
    }
    incCounter('orchestrator_events_total', { type: event.type, outcome });
    return outcome;
  }

  private async onHead(
    event: PullRequestHeadEvent,
    trigger: ReviewTrigger,
  ): Promise<DispatchOutcome> {
    return this.review(event, trigger, 'standard', undefined, event.prUpdatedAt);
  }

  private async onCommand(event: ReviewCommandEvent): Promise<DispatchOutcome> {
    if (event.command === 'cancel') {
      const target = await this.findPullRequest(event);
      if (!target) return 'ignored_unknown_pull_request';
      await this.withLockRetry(() =>
        this.supersession.cancelActiveRuns(target.repository.organizationId, target.pullRequestId),
      );
      return 'cancelled';
    }
    return this.review(
      event,
      'manual',
      event.command === 'full' ? 'full' : 'standard',
      `manual:${event.commentId}`,
    );
  }

  private async onClosed(event: PullRequestClosedEvent): Promise<DispatchOutcome> {
    const target = await this.findPullRequest(event);
    if (!target) return 'ignored_unknown_pull_request';
    await this.withLockRetry(() =>
      this.supersession.cancelActiveRuns(target.repository.organizationId, target.pullRequestId, {
        closedState: event.merged ? 'merged' : 'closed',
      }),
    );
    return 'cancelled';
  }

  private async review(
    event: PullRequestHeadEvent | ReviewCommandEvent,
    trigger: ReviewTrigger,
    depth: ReviewDepth,
    triggerSuffix?: string,
    eventUpdatedAt?: string,
  ): Promise<DispatchOutcome> {
    const synced = await this.repositories.upsertFromEvent(event.repo);
    if (synced.status === 'unknown_installation') return 'ignored_unknown_installation';
    if (synced.status === 'access_lost') return 'ignored_access_lost';
    const repo = synced.repository;
    if (!repo.enabled || repo.accessState !== 'active') return 'ignored_repository_disabled';

    let pr: ProviderPullRequest;
    try {
      pr = await this.providers.repository(event.pr.provider).getPullRequest(event.pr);
    } catch (err) {
      if (err instanceof ProviderError && err.kind === 'not_found') {
        // Either the PR is gone or the repository access is: a refresh marks the latter.
        const refreshed = await this.repositories.upsertFromEvent(event.repo, { refresh: true });
        return refreshed.status === 'access_lost'
          ? 'ignored_access_lost'
          : 'ignored_unknown_pull_request';
      }
      throw err;
    }

    const upserted = await this.pullRequests.upsert(
      { organizationId: repo.organizationId, repositoryId: repo.id },
      pr,
    );
    if (pr.state !== 'open') return 'pr_not_open';
    const at = providerUpdatedAt(pr) ?? (eventUpdatedAt ? new Date(eventUpdatedAt) : null);
    return (
      await this.withLockRetry(() =>
        this.supersession.startReview({
          organizationId: repo.organizationId,
          pullRequestId: upserted.pullRequestId,
          headSha: pr.headSha,
          baseSha: pr.baseSha,
          trigger,
          depth,
          prUpdatedAt: at,
          triggerSuffix,
        }),
      )
    ).outcome;
  }

  private async findPullRequest(
    event: PullRequestClosedEvent | ReviewCommandEvent,
  ): Promise<{ repository: SyncedRepository; pullRequestId: string } | null> {
    const synced = await this.repositories.upsertFromEvent(event.repo);
    if (synced.status !== 'synced') return null;
    const repository = synced.repository;
    const row = await this.dbs.withTx(repository.organizationId, (trx) =>
      trx
        .selectFrom('pull_requests')
        .select('id')
        .where('repository_id', '=', repository.id)
        .where('provider_number', '=', event.pr.number)
        .executeTakeFirst(),
    );
    return row ? { repository, pullRequestId: row.id } : null;
  }

  /**
   * The webhook is acknowledged before orchestration, so GitHub cannot redeliver on a lock
   * timeout; the orchestrator retries with backoff instead (the reconciler is the backstop).
   */
  private async withLockRetry<T>(fn: () => Promise<T>): Promise<T> {
    for (let attempt = 1; ; attempt++) {
      try {
        return await fn();
      } catch (err) {
        if (!isLockTimeout(err) || attempt >= LOCK_RETRY_ATTEMPTS) throw err;
        incCounter('supersession_lock_retries_total');
        this.logger.warn(`pull request lock busy; retry ${attempt}`);
        await new Promise((r) => setTimeout(r, LOCK_RETRY_BASE_MS * 2 ** (attempt - 1)));
      }
    }
  }
}
