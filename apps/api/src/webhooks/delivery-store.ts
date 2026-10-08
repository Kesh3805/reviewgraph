import { Inject, Injectable, Logger } from '@nestjs/common';
import { trace } from '@opentelemetry/api';
import type { Redis } from 'ioredis';
import { sql } from 'kysely';
import { incCounter } from '../common/metrics';
import { REDIS } from '../common/redis.module';
import { DbService } from '../db/db.module';
import type {
  DeliveryContext,
  DeliveryOutcome,
  DeliveryRecord,
  DeliveryStore,
  DeliveryWorkResult,
} from './webhook.ports';

export const DELIVERY_PROVIDER = 'github';
/** Redis keeps a delivery id for 72 h (GitHub redelivers within that window). */
export const DELIVERY_KEY_TTL_SECONDS = 259_200;
export const deliveryKey = (deliveryId: string): string => `rg:wh:gh:${deliveryId}`;

/**
 * Delivery idempotency (GH-003):
 *  1. fast path: `SET rg:wh:gh:{id} 1 NX EX 259200`. A key that was already set only means
 *     "probably seen": Postgres is asked (cheap read) and has the final word, so a crash between
 *     the Redis write and the commit never loses a delivery;
 *  2. durable truth: `INSERT ... ON CONFLICT (provider, delivery_id) DO NOTHING` in the same
 *     transaction as the work. Two simultaneous deliveries produce one insert; the loser sees
 *     no row and answers duplicate;
 *  3. a failure anywhere rolls the row back (and clears the Redis key) so the retry is fresh.
 * Redis being down only costs the fast path: Postgres alone is sufficient.
 */
@Injectable()
export class PgDeliveryStore implements DeliveryStore {
  private readonly logger = new Logger(PgDeliveryStore.name);

  constructor(
    private readonly dbs: DbService,
    @Inject(REDIS) private readonly redis: Redis,
  ) {}

  async process<A>(
    delivery: DeliveryRecord,
    work: (ctx: DeliveryContext) => Promise<DeliveryWorkResult<A>>,
  ): Promise<DeliveryOutcome<A>> {
    const key = deliveryKey(delivery.deliveryId);
    const claimedInRedis = await this.claim(key);

    if (!claimedInRedis && (await this.existsInPg(delivery.deliveryId))) {
      return this.duplicate();
    }

    let result: DeliveryWorkResult<A> | 'duplicate';
    try {
      result = await this.dbs.withTx(null, async (trx) => {
        const { rows } = await sql<{ inserted: boolean }>`
          select rg_record_webhook_delivery(
            ${DELIVERY_PROVIDER}, ${delivery.deliveryId}, ${delivery.eventName},
            ${delivery.action ?? null}::text, ${delivery.installationId ?? null}::bigint,
            ${delivery.payloadSha256}, true) as inserted`.execute(trx);
        if (!rows[0]?.inserted) return 'duplicate' as const;
        const outcome = await work({ trx });
        await sql`
          select rg_finish_webhook_delivery(
            ${DELIVERY_PROVIDER}, ${delivery.deliveryId}, ${outcome.status}, ${null}::text,
            ${outcome.organizationId ?? null}::uuid)`.execute(trx);
        return outcome;
      });
    } catch (err) {
      // Rolled back: let the retry through even though the fast path said "seen".
      if (claimedInRedis) await this.release(key);
      throw err;
    }

    if (result === 'duplicate') return this.duplicate();
    result.afterCommit?.();
    return { duplicate: false, ack: result.ack };
  }

  async payloadHashOf(deliveryId: string): Promise<string | null> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ hash: string | null }>`
        select rg_webhook_delivery_hash(${DELIVERY_PROVIDER}, ${deliveryId}) as hash`.execute(trx),
    );
    return rows[0]?.hash ?? null;
  }

  private duplicate(): { duplicate: true } {
    incCounter('webhook_duplicates_total');
    trace.getActiveSpan()?.setAttribute('duplicate', true);
    return { duplicate: true };
  }

  /** True when this call set the key (first sight). A Redis failure counts as "not claimed". */
  private async claim(key: string): Promise<boolean> {
    try {
      return (await this.redis.set(key, '1', 'EX', DELIVERY_KEY_TTL_SECONDS, 'NX')) === 'OK';
    } catch {
      this.logger.warn('redis unavailable for webhook idempotency; using postgres only');
      return false;
    }
  }

  private async release(key: string): Promise<void> {
    try {
      await this.redis.del(key);
    } catch {
      // The key expires on its own, and Postgres decides anyway.
    }
  }

  private async existsInPg(deliveryId: string): Promise<boolean> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ seen: boolean }>`
        select rg_webhook_delivery_exists(${DELIVERY_PROVIDER}, ${deliveryId}) as seen`.execute(
        trx,
      ),
    );
    return rows[0]?.seen === true;
  }
}
