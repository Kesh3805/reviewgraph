import { Injectable, Logger, Module } from '@nestjs/common';
import type { ProviderEvent } from '../providers/ports';
import { GithubCommandAcknowledger } from '../providers/github/command-reaction';
import { GithubEventNormalizer } from '../providers/github/event-normalizer.service';
import { GithubModule } from '../providers/github/github.module';
import { GithubWebhookController } from './github-webhook.controller';
import {
  DELIVERY_STORE,
  COMMAND_ACKNOWLEDGER,
  EVENT_NORMALIZER,
  PROVIDER_EVENT_SINK,
  type DeliveryOutcome,
  type DeliveryRecord,
  type DeliveryStore,
  type ProviderEventSink,
} from './webhook.ports';

/**
 * Stand-in until GH-003 lands the Redis SETNX + `webhook_deliveries` store: a bounded in-memory
 * set, so a single process still deduplicates redeliveries.
 */
@Injectable()
export class InMemoryDeliveryStore implements DeliveryStore {
  private readonly seen = new Set<string>();
  private readonly capacity = 10_000;

  record(delivery: DeliveryRecord): Promise<DeliveryOutcome> {
    if (this.seen.has(delivery.deliveryId)) return Promise.resolve('duplicate');
    this.seen.add(delivery.deliveryId);
    if (this.seen.size > this.capacity) {
      const oldest = this.seen.values().next().value as string;
      this.seen.delete(oldest);
    }
    return Promise.resolve('new');
  }
}

/** Placeholder until SUP-001 wires the orchestrator; makes the missing handoff visible. */
@Injectable()
export class UnwiredEventSink implements ProviderEventSink {
  private readonly logger = new Logger(UnwiredEventSink.name);

  dispatch(event: ProviderEvent): Promise<void> {
    this.logger.warn(`no orchestrator wired; dropping ${event.type} delivery=${event.deliveryId}`);
    return Promise.resolve();
  }
}

@Module({
  imports: [GithubModule],
  controllers: [GithubWebhookController],
  providers: [
    { provide: DELIVERY_STORE, useClass: InMemoryDeliveryStore },
    { provide: EVENT_NORMALIZER, useExisting: GithubEventNormalizer },
    { provide: COMMAND_ACKNOWLEDGER, useExisting: GithubCommandAcknowledger },
    { provide: PROVIDER_EVENT_SINK, useClass: UnwiredEventSink },
  ],
  exports: [DELIVERY_STORE, EVENT_NORMALIZER, PROVIDER_EVENT_SINK, COMMAND_ACKNOWLEDGER],
})
export class WebhooksModule {}
