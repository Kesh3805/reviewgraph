import { Injectable, Logger, Module } from '@nestjs/common';
import type { ProviderEvent } from '../providers/ports';
import { GithubCommandAcknowledger } from '../providers/github/command-reaction';
import { GithubEventNormalizer } from '../providers/github/event-normalizer.service';
import { GithubModule } from '../providers/github/github.module';
import { PgDeliveryStore } from './delivery-store';
import { GithubWebhookController } from './github-webhook.controller';
import {
  DELIVERY_STORE,
  COMMAND_ACKNOWLEDGER,
  EVENT_NORMALIZER,
  PROVIDER_EVENT_SINK,
  type ProviderEventSink,
} from './webhook.ports';

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
    { provide: DELIVERY_STORE, useClass: PgDeliveryStore },
    { provide: EVENT_NORMALIZER, useExisting: GithubEventNormalizer },
    { provide: COMMAND_ACKNOWLEDGER, useExisting: GithubCommandAcknowledger },
    { provide: PROVIDER_EVENT_SINK, useClass: UnwiredEventSink },
  ],
  exports: [DELIVERY_STORE, EVENT_NORMALIZER, PROVIDER_EVENT_SINK, COMMAND_ACKNOWLEDGER],
})
export class WebhooksModule {}
