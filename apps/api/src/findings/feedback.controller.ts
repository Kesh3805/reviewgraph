import { Body, Controller, Get, Param, Post, Query } from '@nestjs/common';
import { ApiBody, ApiCreatedResponse, ApiOkResponse, ApiTags } from '@nestjs/swagger';
import {
  CurrentUser,
  Tenant,
  type RequestTenant,
  type RequestUser,
} from '../tenancy/request-context';
import { RequireRole } from '../tenancy/roles.decorator';
import {
  FeedbackListDto,
  FeedbackRequestDto,
  FeedbackResultDto,
  FeedbackSummaryDto,
  FeedbackSummaryQueryDto,
  type FeedbackResponse,
  type FeedbackResult,
  type FeedbackSummary,
} from './dto/feedback.dto';
import { FeedbackService } from './feedback.service';

/**
 * Feedback API (API-012). Any member may give feedback; creating a suppression with it requires
 * `maintainer`. The body is validated here rather than by the global pipe so that an invalid
 * verdict answers 422 (semantic error), not 400.
 */
@ApiTags('feedback')
@Controller()
export class FeedbackController {
  constructor(private readonly feedback: FeedbackService) {}

  @Post('findings/:findingId/feedback')
  @RequireRole('viewer')
  @ApiBody({ type: FeedbackRequestDto })
  @ApiCreatedResponse({ type: FeedbackResultDto })
  submit(
    @Param('findingId') findingId: string,
    @Body() body: Record<string, unknown>,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
  ): Promise<FeedbackResult> {
    const request = this.feedback.parseRequest(body);
    return this.feedback.submit(
      tenant.organizationId,
      { userId: user!.userId, role: tenant.role },
      findingId,
      request,
    );
  }

  @Get('findings/:findingId/feedback')
  @RequireRole('viewer')
  @ApiOkResponse({ type: FeedbackListDto })
  list(
    @Param('findingId') findingId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<{ items: FeedbackResponse[] }> {
    return this.feedback.list(tenant.organizationId, findingId);
  }

  @Get('repositories/:repoId/feedback/summary')
  @RequireRole('viewer')
  @ApiOkResponse({ type: FeedbackSummaryDto })
  summary(
    @Param('repoId') repoId: string,
    @Query() query: FeedbackSummaryQueryDto,
    @Tenant() tenant: RequestTenant,
  ): Promise<FeedbackSummary> {
    return this.feedback.summary(tenant.organizationId, repoId, query.since);
  }
}
