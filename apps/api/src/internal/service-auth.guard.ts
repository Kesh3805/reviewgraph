import {
  CanActivate,
  ExecutionContext,
  HttpException,
  HttpStatus,
  Inject,
  Injectable,
  Logger,
  SetMetadata,
  createParamDecorator,
} from '@nestjs/common';
import { Reflector } from '@nestjs/core';
import type { Request } from 'express';
import { incCounter } from '../common/metrics';
import {
  JTI_TTL_SECONDS,
  ServiceAuthError,
  verifyServiceToken,
  type JtiStore,
  type ServiceAuthFailureReason,
  type ServiceKeyRing,
  type ServiceScope,
  type ServiceTokenClaims,
} from './service-token';

export const SERVICE_KEY_RING = Symbol('SERVICE_KEY_RING');
export const SERVICE_JTI_STORE = Symbol('SERVICE_JTI_STORE');
const POLICY_KEY = 'rg:service-auth-policy';

export interface ServiceAuthPolicy {
  /** Every listed scope must be present in the token. */
  scopes: ServiceScope[];
  /** Route param whose value must equal the token `repo` claim (credential broker: `id`). */
  repoParam?: string;
}

/** Declares the scopes an `/internal/**` route needs. Routes without a policy are denied. */
export const ServiceAuth = (policy: ServiceAuthPolicy): MethodDecorator & ClassDecorator =>
  SetMetadata(POLICY_KEY, policy);

/** Error responses carry a status and nothing else: no reason, no hint. */
class DetailessException extends HttpException {
  constructor(status: HttpStatus) {
    super({}, status);
  }
}

type ServiceRequest = Request & { serviceAuth?: ServiceTokenClaims };

/** The verified claims of the calling service (set by `ServiceAuthGuard`). */
export const ServiceCaller = createParamDecorator(
  (_data: unknown, ctx: ExecutionContext): ServiceTokenClaims | undefined =>
    ctx.switchToHttp().getRequest<ServiceRequest>().serviceAuth,
);

/** Paths guarded by service auth (`/internal/**`, outside the `/api/v1` prefix). */
export function isInternalPath(path: string): boolean {
  return path === '/internal' || path.startsWith('/internal/');
}

/**
 * Registered as a global guard but acts only on `/internal/**`, so a new internal controller is
 * authenticated by construction and a forgotten `@ServiceAuth` policy fails closed.
 * Authentication failures answer 401 with no detail; the reason goes to logs and metrics.
 */
@Injectable()
export class ServiceAuthGuard implements CanActivate {
  private readonly logger = new Logger(ServiceAuthGuard.name);

  constructor(
    private readonly reflector: Reflector,
    @Inject(SERVICE_KEY_RING) private readonly keys: ServiceKeyRing,
    @Inject(SERVICE_JTI_STORE) private readonly jti: JtiStore,
  ) {}

  async canActivate(context: ExecutionContext): Promise<boolean> {
    if (context.getType() !== 'http') return true;
    const req = context.switchToHttp().getRequest<ServiceRequest>();
    if (!isInternalPath(req.path)) return true;

    const route = (req.route as { path?: string } | undefined)?.path ?? 'unmatched';
    const claims = await this.authenticate(req, route);

    const policy = this.reflector.getAllAndOverride<ServiceAuthPolicy | undefined>(POLICY_KEY, [
      context.getHandler(),
      context.getClass(),
    ]);
    if (!policy) throw this.forbid('no_policy', route);
    if (!policy.scopes.every((s) => claims.scope.includes(s))) {
      throw this.forbid('insufficient_scope', route);
    }
    if (policy.repoParam) {
      const expected = req.params?.[policy.repoParam];
      if (!claims.repo || claims.repo !== expected) throw this.forbid('repo_mismatch', route);
    }
    req.serviceAuth = claims;
    return true;
  }

  private async authenticate(req: ServiceRequest, route: string): Promise<ServiceTokenClaims> {
    const header = req.headers.authorization;
    const match = typeof header === 'string' ? /^Bearer\s+(\S+)$/i.exec(header) : null;
    if (!match) throw this.reject('missing_token', route);
    let claims: ServiceTokenClaims;
    try {
      claims = await verifyServiceToken(this.keys, match[1] ?? '', { audience: 'rg-api' });
    } catch (err) {
      throw this.reject(err instanceof ServiceAuthError ? err.reason : 'malformed', route);
    }
    let fresh: boolean;
    try {
      fresh = await this.jti.claim(claims.jti, JTI_TTL_SECONDS);
    } catch {
      // Fail closed: without the replay cache a replay cannot be ruled out.
      incCounter('service_auth_failures_total', { reason: 'replay_store_unavailable', route });
      this.logger.error(`service auth replay store unavailable route=${route}`);
      throw new DetailessException(HttpStatus.SERVICE_UNAVAILABLE);
    }
    if (!fresh) throw this.reject('replayed', route);
    return claims;
  }

  private reject(reason: ServiceAuthFailureReason, route: string): HttpException {
    incCounter('service_auth_failures_total', { reason, route });
    this.logger.warn(`service auth rejected reason=${reason} route=${route}`);
    return new DetailessException(HttpStatus.UNAUTHORIZED);
  }

  private forbid(reason: string, route: string): HttpException {
    incCounter('service_auth_failures_total', { reason, route });
    this.logger.warn(`service auth forbidden reason=${reason} route=${route}`);
    return new DetailessException(HttpStatus.FORBIDDEN);
  }
}
