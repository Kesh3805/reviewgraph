import { Module } from '@nestjs/common';
import { RepositoriesModule } from '../repositories/repositories.module';
import { PullRequestSyncService } from './pull-request-sync.service';
import { PgJobQueue } from '../jobs/pg-job-queue';
import { REVIEW_JOBS } from './review-jobs.port';
import { ReviewOrchestrator } from './review-orchestrator';
import { SupersessionService } from './supersession.service';

/**
 * Review orchestration: pull request sync (GH-005), supersession (SUP-001) and the event
 * orchestrator that the webhook sink and the reconciler feed.
 */
@Module({
  imports: [RepositoriesModule],
  providers: [
    PullRequestSyncService,
    SupersessionService,
    ReviewOrchestrator,
    // The adapter over the shared jobs table (API-007, global JobsModule).
    { provide: REVIEW_JOBS, useExisting: PgJobQueue },
  ],
  exports: [PullRequestSyncService, SupersessionService, ReviewOrchestrator, REVIEW_JOBS],
})
export class ReviewOrchestrationModule {}
