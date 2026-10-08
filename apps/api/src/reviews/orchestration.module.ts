import { Module } from '@nestjs/common';
import { RepositoriesModule } from '../repositories/repositories.module';
import { PullRequestSyncService } from './pull-request-sync.service';
import { REVIEW_JOBS, UnwiredReviewJobs } from './review-jobs.port';
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
    // API-007 replaces this with the adapter over the shared jobs table.
    { provide: REVIEW_JOBS, useClass: UnwiredReviewJobs },
  ],
  exports: [PullRequestSyncService, SupersessionService, ReviewOrchestrator, REVIEW_JOBS],
})
export class ReviewOrchestrationModule {}
