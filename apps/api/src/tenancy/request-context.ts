import { createParamDecorator, type ExecutionContext } from '@nestjs/common';
import type { Request } from 'express';

/** Set by the SessionGuard (API-004) once the session cookie is verified. */
export interface RequestUser {
  userId: string;
  sessionId: string;
}

export type MembershipRole = 'viewer' | 'member' | 'admin' | 'owner';

/** Set by the TenancyGuard: the organization the request acts in and the caller's role there. */
export interface RequestTenant {
  organizationId: string;
  role: MembershipRole;
}

export interface TenantRequest extends Request {
  rgUser?: RequestUser;
  rgTenant?: RequestTenant;
}

/** The authenticated user of the current request (401 is the guard's job, so this may be unset). */
export const CurrentUser = createParamDecorator(
  (_data: unknown, ctx: ExecutionContext): RequestUser | undefined =>
    ctx.switchToHttp().getRequest<TenantRequest>().rgUser,
);

/** The resolved tenant; present on routes that carry `@RequireRole`. */
export const Tenant = createParamDecorator(
  (_data: unknown, ctx: ExecutionContext): RequestTenant => {
    const tenant = ctx.switchToHttp().getRequest<TenantRequest>().rgTenant;
    if (!tenant) throw new Error('Tenant() used on a route without @RequireRole');
    return tenant;
  },
);
