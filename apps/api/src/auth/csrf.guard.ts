import { timingSafeEqual } from 'node:crypto';
import {
  CanActivate,
  ExecutionContext,
  ForbiddenException,
  Inject,
  Injectable,
} from '@nestjs/common';
import { Reflector } from '@nestjs/core';
import { incCounter } from '../common/metrics';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import type { TenantRequest } from '../tenancy/request-context';
import { CSRF_COOKIE, CSRF_HEADER, cookiesOf } from './cookies';
import { skipsSessionAuth } from './session.guard';

const MUTATING = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

/**
 * CSRF protection for every mutating, session-authenticated route (API-004). Two independent
 * checks, both required:
 *  - double submit: the `X-CSRF-Token` header must equal the (non HttpOnly) `rg_csrf` cookie;
 *  - the `Origin` header must equal `WEB_ORIGIN`.
 * This closes the class of bug in the legacy `web.rs`, whose unauthenticated POST routes could
 * be fired from any page.
 */
@Injectable()
export class CsrfGuard implements CanActivate {
  private readonly origin: string;

  constructor(
    private readonly reflector: Reflector,
    @Inject(APP_CONFIG) config: AppConfig,
  ) {
    this.origin = new URL(config.WEB_ORIGIN).origin;
  }

  canActivate(context: ExecutionContext): boolean {
    const req = context.switchToHttp().getRequest<TenantRequest>();
    if (!MUTATING.has(req.method)) return true;
    if (skipsSessionAuth(this.reflector, context)) return true;

    const origin = req.headers.origin;
    if (origin !== this.origin) return this.reject('origin');
    const header = req.headers[CSRF_HEADER];
    const cookie = cookiesOf(req)[CSRF_COOKIE];
    if (typeof header !== 'string' || !cookie || !safeEqual(header, cookie)) {
      return this.reject('token');
    }
    return true;
  }

  private reject(reason: 'origin' | 'token'): never {
    incCounter('csrf_rejections_total', { reason });
    throw new ForbiddenException('csrf check failed');
  }
}

function safeEqual(a: string, b: string): boolean {
  const left = Buffer.from(a);
  const right = Buffer.from(b);
  return left.length === right.length && timingSafeEqual(left, right);
}
