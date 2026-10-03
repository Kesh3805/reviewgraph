import { Controller, Get, Patch } from '@nestjs/common';
import type { NestExpressApplication } from '@nestjs/platform-express';
import type { NextFunction, Response } from 'express';
import { RequireRole } from '../../src/tenancy/roles.decorator';
import { Tenant, type RequestTenant, type TenantRequest } from '../../src/tenancy/request-context';

/** Probe routes: a viewer route and a maintainer route, both scoped by `:repoId`. */
@Controller('probe/repos')
export class TenancyProbeController {
  @Get(':repoId')
  @RequireRole('viewer')
  read(@Tenant() tenant: RequestTenant): RequestTenant {
    return tenant;
  }

  @Patch(':repoId')
  @RequireRole('maintainer')
  write(@Tenant() tenant: RequestTenant): RequestTenant {
    return tenant;
  }

  @Get()
  @RequireRole('viewer')
  list(@Tenant() tenant: RequestTenant): RequestTenant {
    return tenant;
  }
}

/** Stands in for the SessionGuard: `x-test-user` becomes the authenticated user. */
export function installTestAuth(app: NestExpressApplication): void {
  app.use((req: TenantRequest, _res: Response, next: NextFunction) => {
    const userId = req.headers['x-test-user'];
    if (typeof userId === 'string') req.rgUser = { userId, sessionId: 'test-session' };
    next();
  });
}
