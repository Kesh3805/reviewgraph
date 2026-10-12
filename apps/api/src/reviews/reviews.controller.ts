import { Controller, Get, HttpCode, Param, Post, Query } from '@nestjs/common';
import { ApiAcceptedResponse, ApiOkResponse, ApiTags } from '@nestjs/swagger';
import {
  CurrentUser,
  Tenant,
  type RequestTenant,
  type RequestUser,
} from '../tenancy/request-context';
import { RequireRole } from '../tenancy/roles.decorator';
import {
  CancelReviewResultDto,
  ListPullRequestsQueryDto,
  ListReviewsQueryDto,
  PullRequestDto,
  PullRequestListDto,
  ReviewDetailDto,
  ReviewListDto,
  StartReviewResultDto,
  type CancelReviewResult,
  type PullRequestResponse,
  type ReviewDetailResponse,
  type ReviewSummaryResponse,
  type StartReviewResult,
} from './dto/review.dto';
import { ReviewsService } from './reviews.service';

/**
 * Reviews API (API-009, PRD section 106): pull requests, review runs, manual trigger and cancel.
 * `viewer` reads; `maintainer` triggers and cancels. Every route carries a resource id, so the
 * tenancy guard resolves the organization and answers 404 for other tenants.
 */
@ApiTags('reviews')
@Controller()
export class ReviewsController {
  constructor(private readonly reviews: ReviewsService) {}

  @Get('repositories/:repoId/pull-requests')
  @RequireRole('viewer')
  @ApiOkResponse({ type: PullRequestListDto })
  listPullRequests(
    @Param('repoId') repoId: string,
    @Query() query: ListPullRequestsQueryDto,
    @Tenant() tenant: RequestTenant,
  ): Promise<{ items: PullRequestResponse[]; next_cursor: string | null }> {
    return this.reviews.listPullRequests(tenant.organizationId, repoId, query);
  }

  @Get('pull-requests/:pullRequestId')
  @RequireRole('viewer')
  @ApiOkResponse({ type: PullRequestDto })
  getPullRequest(
    @Param('pullRequestId') pullRequestId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<PullRequestResponse> {
    return this.reviews.getPullRequest(tenant.organizationId, pullRequestId);
  }

  /** Starts a review at the current head, superseding the active run (one per minute). */
  @Post('pull-requests/:pullRequestId/review')
  @RequireRole('maintainer')
  @HttpCode(202)
  @ApiAcceptedResponse({ type: StartReviewResultDto })
  startReview(
    @Param('pullRequestId') pullRequestId: string,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
  ): Promise<StartReviewResult> {
    return this.reviews.startManualReview(
      tenant.organizationId,
      { userId: user!.userId },
      pullRequestId,
    );
  }

  @Get('pull-requests/:pullRequestId/reviews')
  @RequireRole('viewer')
  @ApiOkResponse({ type: ReviewListDto })
  listReviews(
    @Param('pullRequestId') pullRequestId: string,
    @Query() query: ListReviewsQueryDto,
    @Tenant() tenant: RequestTenant,
  ): Promise<{ items: ReviewSummaryResponse[]; next_cursor: string | null }> {
    return this.reviews.listReviews(tenant.organizationId, pullRequestId, query);
  }

  @Get('pull-requests/:pullRequestId/reviews/:reviewId')
  @RequireRole('viewer')
  @ApiOkResponse({ type: ReviewDetailDto })
  getReview(
    @Param('pullRequestId') pullRequestId: string,
    @Param('reviewId') reviewId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<ReviewDetailResponse> {
    return this.reviews.getReview(tenant.organizationId, reviewId, pullRequestId);
  }

  /** Cancels an active run and its queued jobs; 409 for a run that already finished. */
  @Post('reviews/:reviewId/cancel')
  @RequireRole('maintainer')
  @HttpCode(200)
  @ApiOkResponse({ type: CancelReviewResultDto })
  cancel(
    @Param('reviewId') reviewId: string,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
  ): Promise<CancelReviewResult> {
    return this.reviews.cancel(tenant.organizationId, { userId: user!.userId }, reviewId);
  }
}
