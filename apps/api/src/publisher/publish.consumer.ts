import { Injectable } from '@nestjs/common';
import { z } from 'zod';
import { PublisherService, type PublishOutcome } from './publisher.service';

export const REVIEW_PUBLISH_QUEUE = 'review-publish';
/** Publish-consumer concurrency (GH-009). */
export const PUBLISH_CONCURRENCY = 4;
/** Job idempotency key of a run's publication. */
export const publishJobKey = (reviewRunId: string): string => `publish:${reviewRunId}`;

/** The `review-publish` payload: ids only (API-007). */
export const reviewPublishPayload = z.object({ review_run_id: z.uuid() }).strict();
export type ReviewPublishPayload = z.infer<typeof reviewPublishPayload>;

/**
 * The `review-publish` job handler. The PIPE-003 transition VERIFYING→PUBLISHING enqueues the
 * job; the API-007 queue adapter registers this handler with `consume(REVIEW_PUBLISH_QUEUE,
 * handler, { concurrency: PUBLISH_CONCURRENCY })`. A resolved promise completes the job (also
 * for `skipped_superseded` and `failed_permanent`, which must not be retried); a thrown
 * `ProviderError` that is transient or rate limited makes the queue retry with backoff.
 */
@Injectable()
export class PublishConsumer {
  constructor(private readonly publisher: PublisherService) {}

  async handle(payload: unknown): Promise<{ outcome: PublishOutcome }> {
    const { review_run_id } = reviewPublishPayload.parse(payload);
    return { outcome: await this.publisher.publish(review_run_id) };
  }
}
