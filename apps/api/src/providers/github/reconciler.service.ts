import { createHash, randomUUID } from 'node:crypto';
import {
  Inject,
  Injectable,
  Logger,
  type OnApplicationBootstrap,
  type OnModuleDestroy,
} from '@nestjs/common';
import { SpanStatusCode, trace } from '@opentelemetry/api';
import type { Redis } from 'ioredis';
import { sql } from 'kysely';
import { incCounter } from '../../common/metrics';
import { REDIS } from '../../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import { DbService } from '../../db/db.module';
import {
  REPOSITORY_SETTINGS,
  type RepositorySettingsPort,
} from '../../repositories/repository-settings.port';
import { TRACER_NAME } from '../../telemetry/tracer.service';
import { PROVIDER_EVENT_SINK, type ProviderEventSink } from '../../webhooks/webhook.ports';
import type { PrRef, PullRequestHeadEvent, RepoRef } from '../ports';
import type { GithubAppAuth } from './app-auth.service';
import { toProviderError } from './errors';
import { GITHUB_APP_AUTH } from './github.tokens';
import { isBotLogin, preReviewGuard } from './guards';

export const RECONCILER_LOCK_KEY = 'rg:lock:reconciler';
export const RECONCILER_LOCK_TTL_SECONDS = 280;
export const etagKey = (repositoryId: string): string => `rg:gh:etag:${repositoryId}`;
export const pollDeliveryId = (repositoryId: string, pr: number, headSha: string): string =>
  `poll:${repositoryId}:${pr}:${headSha}`;

interface Target {
  repository_id: string;
  organization_id: string;
  provider_installation_id: string;
  provider_repo_id: string;
  full_name: string;
}

interface GhOpenPull {
  number: number;
  draft?: boolean;
  updated_at?: string;
  user: { login: string; type?: string } | null;
  head: { sha: string };
  base: { sha: string; ref: string };
}

export interface CycleReport {
  ran: boolean;
  scanned: number;
  synthesized: number;
  notModified: number;
  errors: number;
  rateLimited: boolean;
}

/** Explicit setting wins; otherwise on only in development without a webhook secret. */
export function reconcilerEnabled(config: AppConfig): boolean {
  if (config.RECONCILER_ENABLED !== undefined) return config.RECONCILER_ENABLED;
  return config.NODE_ENV === 'development' && !config.GITHUB_WEBHOOK_SECRET;
}

/**
 * Polling reconciler (GH-012): the fallback for missed webhooks (and the only source of events
 * without public ingress). Every `RECONCILE_INTERVAL_SECONDS`, one instance cluster-wide (Redis
 * lock) lists the open PRs of every reviewable repository with ETag conditional requests (a 304
 * is free of rate limit) and synthesizes a `PullRequestHeadEvent` for each head that has no
 * non-superseded run, through the same orchestration path as webhooks. The synthetic delivery
 * id `poll:{repo}:{pr}:{head}` is deterministic, and a race with a real webhook for the same
 * head ends in one run (SUP-001 idempotency key). When webhooks work, it finds nothing.
 */
@Injectable()
export class GithubReconciler implements OnApplicationBootstrap, OnModuleDestroy {
  private readonly logger = new Logger(GithubReconciler.name);
  private timer?: NodeJS.Timeout;
  private running = false;

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(GITHUB_APP_AUTH) private readonly auth: GithubAppAuth | null,
    @Inject(REDIS) private readonly redis: Redis,
    private readonly dbs: DbService,
    @Inject(PROVIDER_EVENT_SINK) private readonly sink: ProviderEventSink,
    @Inject(REPOSITORY_SETTINGS) private readonly settings: RepositorySettingsPort,
  ) {}

  onApplicationBootstrap(): void {
    if (!this.auth || !reconcilerEnabled(this.config)) return;
    this.timer = setInterval(() => {
      if (this.running) return;
      void this.runCycle().catch((err: unknown) => {
        this.logger.error(`reconcile cycle failed (${err instanceof Error ? err.name : 'error'})`);
      });
    }, this.config.RECONCILE_INTERVAL_SECONDS * 1000);
    this.timer.unref();
  }

  onModuleDestroy(): void {
    if (this.timer) clearInterval(this.timer);
  }

  /** One cycle; returns `ran: false` when another instance holds the lock. */
  async runCycle(): Promise<CycleReport> {
    const report: CycleReport = {
      ran: false,
      scanned: 0,
      synthesized: 0,
      notModified: 0,
      errors: 0,
      rateLimited: false,
    };
    if (!this.auth) return report;
    const token = randomUUID();
    const locked =
      (await this.redis.set(
        RECONCILER_LOCK_KEY,
        token,
        'EX',
        RECONCILER_LOCK_TTL_SECONDS,
        'NX',
      )) === 'OK';
    if (!locked) return report;
    this.running = true;
    report.ran = true;
    try {
      await trace.getTracer(TRACER_NAME).startActiveSpan('reconcile_cycle', async (span) => {
        try {
          await this.cycle(report);
          span.setAttributes({
            scanned: report.scanned,
            synthesized: report.synthesized,
            rate_limited: report.rateLimited,
          });
        } catch (err) {
          span.setStatus({ code: SpanStatusCode.ERROR });
          throw err;
        } finally {
          span.end();
        }
      });
    } finally {
      this.running = false;
      if ((await this.redis.get(RECONCILER_LOCK_KEY)) === token) {
        await this.redis.del(RECONCILER_LOCK_KEY);
      }
    }
    return report;
  }

  private async cycle(report: CycleReport): Promise<void> {
    const { rows: targets } = await this.dbs.withTx(null, (trx) =>
      sql<Target>`select * from rg_reconcile_targets('github')`.execute(trx),
    );
    for (const target of targets) {
      try {
        await this.scan(target, report);
        report.scanned += 1;
        incCounter('reconciler_repos_scanned_total');
      } catch (err) {
        const error = toProviderError(err, 'reconcile');
        if (error.kind === 'rate_limited') {
          // Stop early; the next tick resumes.
          report.rateLimited = true;
          this.logger.warn('reconciler rate limited; stopping this cycle');
          return;
        }
        report.errors += 1;
        this.logger.warn(`reconcile failed repository=${target.repository_id} kind=${error.kind}`);
      }
    }
  }

  private async scan(target: Target, report: CycleReport): Promise<void> {
    const slash = target.full_name.indexOf('/');
    const repo: RepoRef = {
      provider: 'github',
      installationId: String(target.provider_installation_id),
      owner: target.full_name.slice(0, slash),
      name: target.full_name.slice(slash + 1),
    };
    const octokit = await this.auth!.getOctokit(repo.installationId);
    const etag = await this.redis.get(etagKey(target.repository_id)).catch(() => null);
    let pulls: GhOpenPull[];
    try {
      const res = await octokit.request('GET /repos/{owner}/{repo}/pulls', {
        owner: repo.owner,
        repo: repo.name,
        state: 'open',
        per_page: 100,
        headers: etag ? { 'if-none-match': etag } : {},
      });
      incCounter('github_api_calls_total', {
        route: 'GET /repos/{owner}/{repo}/pulls',
        status: res.status,
      });
      pulls = res.data as unknown as GhOpenPull[];
      const next = res.headers.etag;
      if (typeof next === 'string') {
        await this.redis.set(etagKey(target.repository_id), next, 'EX', 86_400).catch(() => null);
      }
    } catch (err) {
      if ((err as { status?: unknown }).status === 304) {
        incCounter('github_api_calls_total', {
          route: 'GET /repos/{owner}/{repo}/pulls',
          status: 304,
        });
        report.notModified += 1;
        return;
      }
      throw err;
    }

    const settings = await this.settings.getSettings(repo);
    for (const pull of pulls) {
      if (await this.known(target, pull)) continue;
      const author = {
        login: pull.user?.login ?? 'ghost',
        isBot: pull.user?.type === 'Bot' || isBotLogin(pull.user?.login ?? ''),
      };
      if (
        preReviewGuard({ author, draft: pull.draft === true, baseRef: pull.base.ref }, settings)
      ) {
        continue;
      }
      const pr: PrRef = { ...repo, number: pull.number };
      const deliveryId = pollDeliveryId(target.repository_id, pull.number, pull.head.sha);
      const event: PullRequestHeadEvent = {
        type: 'pull_request_head',
        kind: 'synchronize',
        provider: 'github',
        deliveryId,
        installationId: repo.installationId,
        repo,
        pr,
        headSha: pull.head.sha,
        baseSha: pull.base.sha,
        baseRef: pull.base.ref,
        author,
        draft: pull.draft === true,
        ...(pull.updated_at ? { prUpdatedAt: pull.updated_at } : {}),
      };
      await this.record(target, deliveryId);
      try {
        await this.sink.dispatch(event);
        await this.finish(deliveryId, 'processed', target.organization_id);
      } catch (err) {
        await this.finish(deliveryId, 'failed', target.organization_id);
        throw err;
      }
      report.synthesized += 1;
      incCounter('reconciler_events_synthesized_total');
    }
  }

  /** The head is known when the PR row has it and a non-superseded run exists for it. */
  private async known(target: Target, pull: GhOpenPull): Promise<boolean> {
    const row = await this.dbs.withTx(target.organization_id, (trx) =>
      trx
        .selectFrom('pull_requests as pr')
        .select([
          'pr.head_sha',
          (eb) =>
            eb
              .exists(
                eb
                  .selectFrom('review_runs as rr')
                  .select('rr.id')
                  .whereRef('rr.pull_request_id', '=', 'pr.id')
                  .where('rr.head_sha', '=', pull.head.sha)
                  .where('rr.state', '<>', 'SUPERSEDED'),
              )
              .as('has_run'),
        ])
        .where('pr.repository_id', '=', target.repository_id)
        .where('pr.provider_number', '=', pull.number)
        .executeTakeFirst(),
    );
    return row?.head_sha === pull.head.sha && row.has_run === true;
  }

  /** Synthetic deliveries are logged like webhooks (`event = 'poll'`), payload hash only. */
  private async record(target: Target, deliveryId: string): Promise<void> {
    const hash = createHash('sha256').update(deliveryId).digest('hex');
    await this.dbs.withTx(null, (trx) =>
      sql`select rg_record_webhook_delivery(
            'github', ${deliveryId}, 'poll', ${null}::text,
            ${target.provider_installation_id}::bigint, ${hash}, true)`.execute(trx),
    );
  }

  private async finish(deliveryId: string, status: string, organizationId: string): Promise<void> {
    await this.dbs.withTx(null, (trx) =>
      sql`select rg_finish_webhook_delivery(
            'github', ${deliveryId}, ${status}, ${null}::text, ${organizationId}::uuid)`.execute(
        trx,
      ),
    );
  }
}
