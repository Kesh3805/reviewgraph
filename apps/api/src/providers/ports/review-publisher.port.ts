import type {
  CheckRunRequest,
  ExistingReview,
  PrRef,
  PublishRequest,
  PublishResult,
  ResolveResult,
} from './types';

/**
 * Write side of a code-hosting provider. The MVP can only leave `COMMENT` reviews and check
 * runs: there is no approve, request-changes or merge operation on this port (INV-011/012).
 */
export interface ReviewPublisher {
  /** Posts ONE atomic review (summary plus inline comments). The event is always `COMMENT`. */
  publishReview(req: PublishRequest): Promise<PublishResult>;
  upsertCheckRun(req: CheckRunRequest): Promise<{ checkRunId: string }>;
  /** Looks up a review carrying `marker`, so a retried publish stays idempotent (GH-009). */
  findExistingReview(ref: PrRef, marker: string): Promise<ExistingReview | null>;
  resolveThreads(ref: PrRef, providerCommentIds: string[]): Promise<ResolveResult>;
}
