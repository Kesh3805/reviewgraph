import { Inject, Injectable, Logger, type OnApplicationBootstrap } from '@nestjs/common';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { JOB_QUEUE, type ConsumerHandle, type JobQueue } from '../jobs/job-queue';
import { PUBLISH_CONCURRENCY, PublishConsumer, REVIEW_PUBLISH_QUEUE } from './publish.consumer';

/**
 * Registers `PublishConsumer.handle` on the `review-publish` queue (API-007 wiring of GH-009)
 * with concurrency 4. The queue adapter stops the consumer and releases its unfinished jobs on
 * shutdown. Disabled under NODE_ENV=test unless QUEUE_CONSUMERS_ENABLED=true.
 */
@Injectable()
export class PublishWorker implements OnApplicationBootstrap {
  private readonly logger = new Logger(PublishWorker.name);
  private handle: ConsumerHandle | null = null;

  constructor(
    @Inject(JOB_QUEUE) private readonly queue: JobQueue,
    private readonly consumer: PublishConsumer,
    @Inject(APP_CONFIG) private readonly config: AppConfig,
  ) {}

  get running(): boolean {
    return this.handle !== null;
  }

  onApplicationBootstrap(): void {
    const enabled = this.config.QUEUE_CONSUMERS_ENABLED ?? this.config.NODE_ENV !== 'test';
    if (!enabled) return;
    this.handle = this.queue.consume(
      REVIEW_PUBLISH_QUEUE,
      async (job) => {
        const { outcome } = await this.consumer.handle(job.payload);
        this.logger.log(`publish job=${job.jobId} outcome=${outcome}`);
      },
      { concurrency: PUBLISH_CONCURRENCY },
    );
  }
}
