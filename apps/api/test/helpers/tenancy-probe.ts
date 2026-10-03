import { Controller, Get, Patch } from '@nestjs/common';
import { RequireRole } from '../../src/tenancy/roles.decorator';
import { Tenant, type RequestTenant } from '../../src/tenancy/request-context';

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

/**
 * Stands in for SessionService: the cookie `rg_session=test:<userId>` authenticates that user.
 * The real session, CSRF and tenancy guards stay in place, so their ordering is exercised too.
 */
export const testSessions = {
  verify: (token: string) =>
    Promise.resolve(
      token.startsWith('test:') ? { userId: token.slice(5), sessionId: 'test-session' } : null,
    ),
};

export const asUser = (userId: string): string => `rg_session=test:${userId}; rg_csrf=csrf`;

/** Headers that satisfy the CSRF guard for a mutation. */
export const CSRF_HEADERS = { origin: 'http://localhost:3000', 'x-csrf-token': 'csrf' };
