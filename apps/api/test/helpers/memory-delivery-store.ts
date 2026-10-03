import type { Tx } from '../../src/db/tx';
import type {
  DeliveryContext,
  DeliveryOutcome,
  DeliveryRecord,
  DeliveryStore,
  DeliveryWorkResult,
} from '../../src/webhooks/webhook.ports';

/**
 * Test double for the delivery store: deduplicates in memory and runs the work without a
 * database. The real store (Redis + Postgres) is covered by the integration tests.
 */
export class MemoryDeliveryStore implements DeliveryStore {
  readonly seen = new Set<string>();

  async process<A>(
    delivery: DeliveryRecord,
    work: (ctx: DeliveryContext) => Promise<DeliveryWorkResult<A>>,
  ): Promise<DeliveryOutcome<A>> {
    if (this.seen.has(delivery.deliveryId)) return { duplicate: true };
    this.seen.add(delivery.deliveryId);
    try {
      const result = await work({ trx: undefined as unknown as Tx });
      result.afterCommit?.();
      return { duplicate: false, ack: result.ack };
    } catch (err) {
      this.seen.delete(delivery.deliveryId);
      throw err;
    }
  }
}
