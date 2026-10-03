import {
  BadRequestException,
  CanActivate,
  ExecutionContext,
  ForbiddenException,
  Injectable,
  NotFoundException,
  UnauthorizedException,
} from '@nestjs/common';
import { Reflector } from '@nestjs/core';
import { incCounter } from '../common/metrics';
import { MembershipService, type ResourceKind } from './membership.service';
import type { TenantRequest } from './request-context';
import { REQUIRE_ROLE_KEY, satisfies, type RequiredRole } from './roles.decorator';

/** Route params that identify a tenant resource, and what they identify. */
export const TENANT_PARAMS: ReadonlyArray<readonly [string, ResourceKind]> = [
  ['repoId', 'repository'],
  ['reviewId', 'review'],
  ['findingId', 'finding'],
  ['pullRequestId', 'pull_request'],
  ['installationId', 'installation'],
  ['organizationId', 'organization'],
];

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Tenancy guard (API-003). For a route with `@RequireRole`:
 *  1. resolves the organization from a resource route param (`:repoId`, `:reviewId`, ...),
 *     else from `organization_id` in the query/body, else from the only membership of the caller;
 *  2. checks the membership and role of the caller;
 *  3. stores `{organizationId, role}` on the request for `@Tenant()` and `DbService.withTx`.
 * An unknown id and a foreign id are indistinguishable: both answer 404, never 403, so ids
 * cannot be probed. 403 is returned only when the caller is a member with too low a role.
 */
@Injectable()
export class TenancyGuard implements CanActivate {
  constructor(
    private readonly reflector: Reflector,
    private readonly memberships: MembershipService,
  ) {}

  async canActivate(context: ExecutionContext): Promise<boolean> {
    const required = this.reflector.getAllAndOverride<RequiredRole | undefined>(REQUIRE_ROLE_KEY, [
      context.getHandler(),
      context.getClass(),
    ]);
    if (!required) return true;

    const req = context.switchToHttp().getRequest<TenantRequest>();
    const user = req.rgUser;
    if (!user) throw new UnauthorizedException();

    const organizationId = await this.resolveOrganization(req, user.userId);
    const role = await this.memberships.roleOf(user.userId, organizationId);
    if (!role) throw this.notFound();
    if (!satisfies(role, required)) {
      incCounter('tenancy_denied_total', { reason: 'role' });
      throw new ForbiddenException('insufficient role');
    }
    req.rgTenant = { organizationId, role };
    return true;
  }

  private async resolveOrganization(req: TenantRequest, userId: string): Promise<string> {
    for (const [param, kind] of TENANT_PARAMS) {
      const id = req.params[param];
      if (typeof id !== 'string') continue;
      if (!UUID.test(id)) throw this.notFound();
      const org = await this.memberships.resolveOrg(kind, id);
      if (!org) throw this.notFound();
      return org;
    }
    const explicit = explicitOrganization(req);
    if (explicit !== undefined) {
      if (!UUID.test(explicit)) throw this.notFound();
      return explicit;
    }
    const mine = await this.memberships.listForUser(userId);
    if (mine.length === 1 && mine[0]) return mine[0].organizationId;
    if (mine.length === 0) throw this.notFound();
    throw new BadRequestException('organization_id is required for users in several organizations');
  }

  private notFound(): NotFoundException {
    incCounter('tenancy_denied_total', { reason: 'not_member' });
    return new NotFoundException();
  }
}

function explicitOrganization(req: TenantRequest): string | undefined {
  const fromQuery = req.query?.organization_id;
  if (typeof fromQuery === 'string') return fromQuery;
  const body = req.body as { organization_id?: unknown } | undefined;
  return typeof body?.organization_id === 'string' ? body.organization_id : undefined;
}
