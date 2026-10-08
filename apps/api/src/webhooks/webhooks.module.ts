import { Module } from '@nestjs/common';
import { GithubCommandAcknowledger } from '../providers/github/command-reaction';
import { GithubEventNormalizer } from '../providers/github/event-normalizer.service';
import { GithubModule } from '../providers/github/github.module';
import { InstallationService } from '../providers/github/installation.service';
import { GithubReconciler } from '../providers/github/reconciler.service';
import { RepositoriesModule } from '../repositories/repositories.module';
import { ReviewOrchestrationModule } from '../reviews/orchestration.module';
import { ReviewOrchestrator } from '../reviews/review-orchestrator';
import { PgDeliveryStore } from './delivery-store';
import { GithubWebhookController } from './github-webhook.controller';
import {
  DELIVERY_STORE,
  COMMAND_ACKNOWLEDGER,
  EVENT_NORMALIZER,
  INSTALLATION_LIFECYCLE,
  PROVIDER_EVENT_SINK,
} from './webhook.ports';

@Module({
  imports: [GithubModule, RepositoriesModule, ReviewOrchestrationModule],
  controllers: [GithubWebhookController],
  providers: [
    // GH-012: feeds the same sink as the webhook endpoint, so it lives next to it.
    GithubReconciler,
    { provide: DELIVERY_STORE, useClass: PgDeliveryStore },
    { provide: EVENT_NORMALIZER, useExisting: GithubEventNormalizer },
    { provide: COMMAND_ACKNOWLEDGER, useExisting: GithubCommandAcknowledger },
    { provide: INSTALLATION_LIFECYCLE, useExisting: InstallationService },
    // SUP-001: accepted events go to the review orchestrator (detached from the ack).
    { provide: PROVIDER_EVENT_SINK, useExisting: ReviewOrchestrator },
  ],
  exports: [
    DELIVERY_STORE,
    EVENT_NORMALIZER,
    PROVIDER_EVENT_SINK,
    COMMAND_ACKNOWLEDGER,
    GithubReconciler,
  ],
})
export class WebhooksModule {}
