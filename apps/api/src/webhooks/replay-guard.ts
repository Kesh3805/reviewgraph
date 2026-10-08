import { Inject, Injectable, Logger } from '@nestjs/common';
import type { Redis } from 'ioredis';
import { sql } from 'kysely';
import { AuditService } from '../audit/audit.service';
import { incCounter } from '../common/metrics';
import { REDIS } from '../common/redis.module';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { DbService } from '../db/db.module';
import { DELIVERY_STORE, type DeliveryRecord, type DeliveryStore } from './webhook.ports';

export type ReplayVerdict =
  | { kind: 'fresh' }
  /** Older than `WEBHOOK_MAX_EVENT_AGE_SECONDS`: recorded, acknowledged and ignored. */
  | { kind: 'stale' }
  /** A known delivery id with a different body: never processed (GitHub redelivers the same body). */
  | { kind: 'delivery_id_reuse' }
  /** Over the per-installation intake limit: 429, GitHub retries later. */
  | { kind: 'rate_limited'; retryAfterSeconds: number };

const RATE_WINDOW_SECONDS = 60;
export const rateKey = (installationId: string, window: number): string =>
  `rg:wh:rate:${installationId}:${window}`;

/**
 * The newest timestamp the event itself carries (GitHub signs the body, so these cannot be
 * altered after signing): `pull_request.updated_at`, `head_commit.timestamp`,
 * `check_suite.updated_at`. Events without one (installation, comments) have no age.
 */
export function eventTimestamp(payload: unknown): Date | null {
  const p = payload as {
    pull_request?: { updated_at?: unknown };
    head_commit?: { timestamp?: unknown };
    check_suite?: { updated_at?: unknown };
  } | null;
  const candidates = [
    p?.pull_request?.updated_at,
    p?.head_commit?.timestamp,
    p?.check_suite?.updated_at,
  ];
  let newest: Date | null = null;
  for (const c of candidates) {
    if (typeof c !== 'string') continue;
    const at = new Date(c);
    if (Number.isNaN(at.getTime())) continue;
    if (!newest || at > newest) newest = at;
  }
  return newest;
}

/**
 * Webhook replay protection (SEC-006), run only after a valid signature (so unauthenticated
 * callers cannot probe it) and before the idempotency record:
 *  1. per-installation intake limit (Redis fixed window, `WEBHOOK_INSTALLATION_RATE_LIMIT`/min);
 *  2. delivery id reuse: a recorded delivery id whose stored body hash differs is a replay with a
 *     substituted body (GitHub redelivery reuses the same payload) and is audited;
 *  3. freshness: an event older than `WEBHOOK_MAX_EVENT_AGE_SECONDS` is acknowledged and ignored.
 * A same-body duplicate is left to the idempotency store (GH-003), whose table outlives the Redis
 * TTL. The guard fails open: its own errors never reject a valid first delivery.
 */
@Injectable()
export class ReplayGuard {
  private readonly logger = new Logger(ReplayGuard.name);

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(REDIS) private readonly redis: Redis,
    @Inject(DELIVERY_STORE) private readonly deliveries: DeliveryStore,
    private readonly dbs: DbService,
    private readonly audit: AuditService,
  ) {}

  /** The clock (tests move it). */
  now: () => number = Date.now;

  async check(delivery: DeliveryRecord, payload: unknown): Promise<ReplayVerdict> {
    const limited = await this.rateLimited(delivery.installationId);
    if (limited) {
      incCounter('webhook_replays_rejected_total', { reason: 'rate_limited' });
      return limited;
    }

    if (await this.reused(delivery)) {
      incCounter('webhook_replays_rejected_total', { reason: 'delivery_id_reuse' });
      this.logger.warn(
        `webhook delivery id reused with a different body delivery=${delivery.deliveryId} ` +
          `installation=${delivery.installationId ?? '-'}`,
      );
      await this.auditReuse(delivery);
      return { kind: 'delivery_id_reuse' };
    }

    const at = eventTimestamp(payload);
    const maxAgeMs = this.config.WEBHOOK_MAX_EVENT_AGE_SECONDS * 1000;
    if (at && this.now() - at.getTime() > maxAgeMs) {
      incCounter('webhook_stale_events_total');
      return { kind: 'stale' };
    }
    return { kind: 'fresh' };
  }

  private async rateLimited(
    installationId: string | undefined,
  ): Promise<Extract<ReplayVerdict, { kind: 'rate_limited' }> | null> {
    const limit = this.config.WEBHOOK_INSTALLATION_RATE_LIMIT;
    if (!installationId || limit <= 0) return null;
    const nowS = Math.floor(this.now() / 1000);
    const window = Math.floor(nowS / RATE_WINDOW_SECONDS);
    try {
      const key = rateKey(installationId, window);
      const count = await this.redis.incr(key);
      if (count === 1) await this.redis.expire(key, RATE_WINDOW_SECONDS * 2);
      if (count <= limit) return null;
      return {
        kind: 'rate_limited',
        retryAfterSeconds: (window + 1) * RATE_WINDOW_SECONDS - nowS,
      };
    } catch {
      // Redis down: no intake limit rather than no intake.
      return null;
    }
  }

  private async reused(delivery: DeliveryRecord): Promise<boolean> {
    if (!this.deliveries.payloadHashOf) return false;
    try {
      const stored = await this.deliveries.payloadHashOf(delivery.deliveryId);
      return stored !== null && stored !== delivery.payloadSha256;
    } catch {
      this.logger.warn(`replay lookup unavailable delivery=${delivery.deliveryId}`);
      return false;
    }
  }

  private async auditReuse(delivery: DeliveryRecord): Promise<void> {
    if (!delivery.installationId || !/^\d+$/.test(delivery.installationId)) return;
    try {
      await this.dbs.withTx(null, async (trx) => {
        const { rows } = await sql<{ org: string | null }>`
          select rg_installation_org('github', ${delivery.installationId}::bigint) as org`.execute(
          trx,
        );
        const org = rows[0]?.org;
        if (!org) return;
        await sql`select set_config('app.organization_id', ${org}, true)`.execute(trx);
        await this.audit.record(trx, {
          organizationId: org,
          actor: { type: 'system', id: 'webhook-replay-guard' },
          action: 'webhook.delivery_id_reuse',
          targetType: 'webhook_delivery',
          targetId: delivery.deliveryId,
          outcome: 'denied',
          metadata: {
            severity: 'warning',
            event: delivery.eventName,
            installation_id: delivery.installationId,
          },
        });
      });
    } catch {
      this.logger.warn(`replay audit failed delivery=${delivery.deliveryId}`);
    }
  }
}
