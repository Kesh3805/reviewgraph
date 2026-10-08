import { Controller, Get, Param, Query } from '@nestjs/common';
import { ApiOkResponse, ApiTags } from '@nestjs/swagger';
import { Tenant, type RequestTenant } from '../tenancy/request-context';
import { RequireRole } from '../tenancy/roles.decorator';
import {
  FindingDetailDto,
  FindingListDto,
  FindingTraceDto,
  ListFindingsQueryDto,
  type FindingDetail,
  type FindingSummary,
  type FindingTrace,
} from './dto/finding.dto';
import { FindingsService } from './findings.service';
import { FindingTraceService } from './trace.service';

/**
 * Findings API (API-010): findings of a review by lifecycle state, finding detail and the
 * explainability trace. Read-only; `viewer` and above. Unknown and foreign ids answer 404.
 */
@ApiTags('findings')
@Controller()
export class FindingsController {
  constructor(
    private readonly findings: FindingsService,
    private readonly traces: FindingTraceService,
  ) {}

  @Get('reviews/:reviewId/findings')
  @RequireRole('viewer')
  @ApiOkResponse({ type: FindingListDto })
  list(
    @Param('reviewId') reviewId: string,
    @Query() query: ListFindingsQueryDto,
    @Tenant() tenant: RequestTenant,
  ): Promise<{ items: FindingSummary[] }> {
    return this.findings.listForReview(tenant.organizationId, reviewId, query);
  }

  @Get('findings/:findingId')
  @RequireRole('viewer')
  @ApiOkResponse({ type: FindingDetailDto })
  detail(
    @Param('findingId') findingId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<FindingDetail> {
    return this.findings.detail(tenant.organizationId, findingId);
  }

  @Get('findings/:findingId/trace')
  @RequireRole('viewer')
  @ApiOkResponse({ type: FindingTraceDto })
  trace(
    @Param('findingId') findingId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<FindingTrace> {
    return this.traces.trace(tenant.organizationId, findingId);
  }
}
