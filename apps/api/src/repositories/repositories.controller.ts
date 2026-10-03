import { Body, Controller, Get, HttpCode, Param, Patch, Post, Query } from '@nestjs/common';
import { ApiAcceptedResponse, ApiCreatedResponse, ApiOkResponse, ApiTags } from '@nestjs/swagger';
import {
  CurrentUser,
  Tenant,
  type RequestTenant,
  type RequestUser,
} from '../tenancy/request-context';
import { RequireRole } from '../tenancy/roles.decorator';
import {
  CreateRepositoryDto,
  IndexRequestDto,
  ListRepositoriesQueryDto,
  RepositoryDto,
  RepositoryListDto,
  RepositorySettingsDto,
  RepositoryStatusDto,
  UpdateRepositorySettingsDto,
  type RepositoryResponse,
  type RepositorySettingsResponse,
  type RepositoryStatusResponse,
} from './dto/repository.dto';
import { RepositoriesService } from './repositories.service';

/**
 * Repositories API (PRD section 106). `viewer` can read; `maintainer` can enable, initialize,
 * rebuild and change settings. Routes carry `:repoId` so the tenancy guard resolves the
 * organization and answers 404 for repositories of other organizations.
 */
@ApiTags('repositories')
@Controller('repositories')
export class RepositoriesController {
  constructor(private readonly repositories: RepositoriesService) {}

  /** Enables an installation repository for review. */
  @Post()
  @RequireRole('maintainer')
  @ApiCreatedResponse({ type: RepositoryDto })
  create(
    @Body() body: CreateRepositoryDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
  ): Promise<RepositoryResponse> {
    return this.repositories.enable(
      tenant.organizationId,
      { userId: user!.userId },
      { installationId: body.installation_id, fullName: body.full_name },
    );
  }

  @Get()
  @RequireRole('viewer')
  @ApiOkResponse({ type: RepositoryListDto })
  list(
    @Query() query: ListRepositoriesQueryDto,
    @Tenant() tenant: RequestTenant,
  ): Promise<{ items: RepositoryResponse[]; next_cursor: string | null }> {
    return this.repositories.list(tenant.organizationId, query);
  }

  @Get(':repoId')
  @RequireRole('viewer')
  @ApiOkResponse({ type: RepositoryDto })
  get(
    @Param('repoId') repoId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<RepositoryResponse> {
    return this.repositories.get(tenant.organizationId, repoId);
  }

  /** Enqueues `repository-index` for the default-branch head; once per head. */
  @Post(':repoId/initialize')
  @RequireRole('maintainer')
  @HttpCode(202)
  @ApiAcceptedResponse({ type: IndexRequestDto })
  initialize(@Param('repoId') repoId: string, @Tenant() tenant: RequestTenant) {
    return this.repositories.initialize(tenant.organizationId, repoId);
  }

  @Get(':repoId/status')
  @RequireRole('viewer')
  @ApiOkResponse({ type: RepositoryStatusDto })
  status(
    @Param('repoId') repoId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<RepositoryStatusResponse> {
    return this.repositories.status(tenant.organizationId, repoId);
  }

  /** The PROF-001 profile JSON; 404 before the first index. */
  @Get(':repoId/profile')
  @RequireRole('viewer')
  @ApiOkResponse({ description: 'The repository profile (PROF-001).' })
  profile(@Param('repoId') repoId: string, @Tenant() tenant: RequestTenant): Promise<unknown> {
    return this.repositories.profile(tenant.organizationId, repoId);
  }

  /** Enqueues a forced full index; at most one per repository per hour. */
  @Post(':repoId/graph/rebuild')
  @RequireRole('maintainer')
  @HttpCode(202)
  @ApiAcceptedResponse({ type: IndexRequestDto })
  rebuild(@Param('repoId') repoId: string, @Tenant() tenant: RequestTenant) {
    return this.repositories.rebuild(tenant.organizationId, repoId);
  }

  @Patch(':repoId/settings')
  @RequireRole('maintainer')
  @ApiOkResponse({ type: RepositorySettingsDto })
  updateSettings(
    @Param('repoId') repoId: string,
    @Body() body: UpdateRepositorySettingsDto,
    @Tenant() tenant: RequestTenant,
    @CurrentUser() user: RequestUser | undefined,
  ): Promise<RepositorySettingsResponse> {
    return this.repositories.updateSettings(
      tenant.organizationId,
      { userId: user!.userId },
      repoId,
      body,
    );
  }
}
