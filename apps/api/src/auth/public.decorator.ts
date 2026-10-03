import { SetMetadata } from '@nestjs/common';

export const PUBLIC_KEY = 'rg:public';

/**
 * Opts a route (or controller) out of the session and CSRF guards: health probes, the webhook
 * endpoint (HMAC) and the OAuth login/callback. `/internal/**` is service-authenticated instead
 * and is skipped by path.
 */
export const Public = (): MethodDecorator & ClassDecorator => SetMetadata(PUBLIC_KEY, true);
