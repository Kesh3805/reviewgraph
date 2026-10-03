import { CanActivate, ExecutionContext, Injectable, UnauthorizedException } from '@nestjs/common';
import { Reflector } from '@nestjs/core';
import { isInternalPath } from '../internal/service-auth.guard';
import type { TenantRequest } from '../tenancy/request-context';
import { cookiesOf, SESSION_COOKIE } from './cookies';
import { PUBLIC_KEY } from './public.decorator';
import { SessionService } from './session.service';

/** True when the route authenticates some other way (`@Public()`, or `/internal/**`). */
export function skipsSessionAuth(reflector: Reflector, context: ExecutionContext): boolean {
  const req = context.switchToHttp().getRequest<TenantRequest>();
  if (isInternalPath(req.path)) return true;
  return Boolean(
    reflector.getAllAndOverride<boolean | undefined>(PUBLIC_KEY, [
      context.getHandler(),
      context.getClass(),
    ]),
  );
}

/**
 * Every route needs a valid session except `@Public()` ones (health, webhooks, OAuth login) and
 * `/internal/**` (service tokens, API-005). Sets `req.rgUser` for the tenancy guard.
 */
@Injectable()
export class SessionGuard implements CanActivate {
  constructor(
    private readonly reflector: Reflector,
    private readonly sessions: SessionService,
  ) {}

  async canActivate(context: ExecutionContext): Promise<boolean> {
    if (skipsSessionAuth(this.reflector, context)) return true;
    const req = context.switchToHttp().getRequest<TenantRequest>();
    const token = cookiesOf(req)[SESSION_COOKIE];
    if (!token) throw new UnauthorizedException();
    const user = await this.sessions.verify(token);
    if (!user) throw new UnauthorizedException();
    req.rgUser = user;
    return true;
  }
}
