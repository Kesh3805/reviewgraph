import { Body, Controller, Get, HttpCode, Param, Post, Query, Req } from '@nestjs/common';
import { ApiOkResponse, ApiTags } from '@nestjs/swagger';
import type { RequestWithId } from '../common/request-id.middleware';
import {
  CurrentUser,
  Tenant,
  type RequestTenant,
  type RequestUser,
} from '../tenancy/request-context';
import { RequireRole } from '../tenancy/roles.decorator';
import {
  GraphResponseDto,
  NeighborsQueryDto,
  PathQueryDto,
  SnapshotQueryDto,
  SourceExcerptDto,
  SourceQueryDto,
  SubgraphRequestDto,
  SymbolSearchQueryDto,
  type GraphResponse,
  type SourceExcerpt,
} from './dto/graph.dto';
import { GraphService, type GraphCaller } from './graph.service';

/**
 * Graph proxy API (API-011): read-only graph queries for the explorer and Finding Detail,
 * forwarded to the review engine with the tenant scope added server-side. `viewer` and above.
 */
@ApiTags('graph')
@Controller()
export class GraphController {
  constructor(private readonly graph: GraphService) {}

  @Get('repositories/:repoId/graph/symbols')
  @RequireRole('viewer')
  @ApiOkResponse({ type: GraphResponseDto })
  symbols(
    @Param('repoId') repoId: string,
    @Query() query: SymbolSearchQueryDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
    @Req() req: RequestWithId,
  ): Promise<GraphResponse> {
    return this.graph.searchSymbols(caller(tenant, user, req), repoId, query);
  }

  @Get('repositories/:repoId/graph/symbols/:key')
  @RequireRole('viewer')
  @ApiOkResponse({ type: GraphResponseDto })
  symbol(
    @Param('repoId') repoId: string,
    @Param('key') key: string,
    @Query() query: SnapshotQueryDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
    @Req() req: RequestWithId,
  ): Promise<GraphResponse> {
    return this.graph.symbol(caller(tenant, user, req), repoId, key, query.snapshot);
  }

  @Get('repositories/:repoId/graph/symbols/:key/neighbors')
  @RequireRole('viewer')
  @ApiOkResponse({ type: GraphResponseDto })
  neighbors(
    @Param('repoId') repoId: string,
    @Param('key') key: string,
    @Query() query: NeighborsQueryDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
    @Req() req: RequestWithId,
  ): Promise<GraphResponse> {
    return this.graph.neighbors(caller(tenant, user, req), repoId, key, query);
  }

  /** Side-effect free: `{seeds, depth<=3, kinds, max_nodes<=500}`. */
  @Post('repositories/:repoId/graph/subgraph')
  @RequireRole('viewer')
  @HttpCode(200)
  @ApiOkResponse({ type: GraphResponseDto })
  subgraph(
    @Param('repoId') repoId: string,
    @Body() body: SubgraphRequestDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
    @Req() req: RequestWithId,
  ): Promise<GraphResponse> {
    return this.graph.subgraph(caller(tenant, user, req), repoId, body);
  }

  @Get('repositories/:repoId/graph/path')
  @RequireRole('viewer')
  @ApiOkResponse({ type: GraphResponseDto })
  path(
    @Param('repoId') repoId: string,
    @Query() query: PathQueryDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
    @Req() req: RequestWithId,
  ): Promise<GraphResponse> {
    return this.graph.path(caller(tenant, user, req), repoId, query);
  }

  @Get('reviews/:reviewId/impact/:symbolKey')
  @RequireRole('viewer')
  @ApiOkResponse({ type: GraphResponseDto })
  impact(
    @Param('reviewId') reviewId: string,
    @Param('symbolKey') symbolKey: string,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
    @Req() req: RequestWithId,
  ): Promise<GraphResponse> {
    return this.graph.impact(caller(tenant, user, req), reviewId, symbolKey);
  }

  /** A redacted excerpt of at most 200 lines; every request is audited. */
  @Get('repositories/:repoId/source')
  @RequireRole('viewer')
  @ApiOkResponse({ type: SourceExcerptDto })
  source(
    @Param('repoId') repoId: string,
    @Query() query: SourceQueryDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
    @Req() req: RequestWithId,
  ): Promise<SourceExcerpt> {
    return this.graph.source(caller(tenant, user, req), repoId, query);
  }
}

function caller(
  tenant: RequestTenant,
  user: RequestUser | undefined,
  req: RequestWithId,
): GraphCaller {
  return { organizationId: tenant.organizationId, userId: user!.userId, requestId: req.requestId };
}
