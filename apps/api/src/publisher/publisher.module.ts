import { Module } from '@nestjs/common';
import { PublishConsumer } from './publish.consumer';
import { PublishWorker } from './publish.worker';
import { PublishGate } from './publish-gate';
import { PublishSource } from './publish-source';
import { PublisherService } from './publisher.service';
import { StaleResolutionService } from './stale-resolution.service';

/** Publication (GH-009), the publish-time gate (SUP-003) and stale resolution (GH-011). */
@Module({
  providers: [
    PublishGate,
    PublishSource,
    StaleResolutionService,
    PublisherService,
    PublishConsumer,
    PublishWorker,
  ],
  exports: [PublisherService, PublishConsumer, PublishGate],
})
export class PublisherModule {}
