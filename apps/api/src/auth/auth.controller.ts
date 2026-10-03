import {
  BadRequestException,
  Controller,
  Get,
  HttpCode,
  Inject,
  Logger,
  NotFoundException,
  Post,
  Query,
  Req,
  Res,
  UnauthorizedException,
} from '@nestjs/common';
import type { Request, Response } from 'express';
import { incCounter } from '../common/metrics';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { CurrentUser, type RequestUser } from '../tenancy/request-context';
import { AuthService, newCsrfToken, OAUTH_STATE_TTL_SECONDS } from './auth.service';
import {
  CSRF_COOKIE,
  OAUTH_STATE_COOKIE,
  SESSION_COOKIE,
  SESSION_TTL_SECONDS,
  clearCookie,
  cookieOptions,
  cookiesOf,
} from './cookies';
import { GithubUnavailableError, OAuthCodeError } from './github-oauth.client';
import { Public } from './public.decorator';
import { SessionService } from './session.service';

const STATE_COOKIE_PATH = '/api/v1/auth/github';

@Controller('auth')
export class AuthController {
  private readonly logger = new Logger(AuthController.name);

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    private readonly auth: AuthService,
    private readonly sessions: SessionService,
  ) {}

  /** Redirects to GitHub with a single-use `state` and a PKCE S256 challenge. */
  @Public()
  @Get('github/login')
  async login(@Res() res: Response): Promise<void> {
    this.requireGithub();
    const { authorizeUrl, stateCookie } = await this.auth.start();
    res.cookie(
      OAUTH_STATE_COOKIE,
      stateCookie,
      cookieOptions(this.config.NODE_ENV, {
        path: STATE_COOKIE_PATH,
        maxAge: OAUTH_STATE_TTL_SECONDS * 1000,
      }),
    );
    res.redirect(302, authorizeUrl);
  }

  @Public()
  @Get('github/callback')
  async callback(
    @Req() req: Request,
    @Res() res: Response,
    @Query('code') code?: string,
    @Query('state') state?: string,
    @Query('error') providerError?: string,
  ): Promise<void> {
    this.requireGithub();
    const stateCookie = cookiesOf(req)[OAUTH_STATE_COOKIE];
    const verifier = await this.auth.consumeState(
      stateCookie,
      typeof state === 'string' ? state : undefined,
    );
    // The state is consumed whatever happens next, so this cookie is never useful again.
    clearCookie(
      res,
      OAUTH_STATE_COOKIE,
      cookieOptions(this.config.NODE_ENV, { path: STATE_COOKIE_PATH }),
    );
    if (verifier === null) {
      incCounter('auth_logins_total', { result: 'state_invalid' });
      throw new BadRequestException('invalid or reused OAuth state');
    }
    if (typeof providerError === 'string') {
      incCounter('auth_logins_total', { result: 'denied' });
      this.redirectToLogin(res, 'access_denied');
      return;
    }
    if (typeof code !== 'string' || !code) {
      incCounter('auth_logins_total', { result: 'invalid_request' });
      throw new BadRequestException('missing authorization code');
    }

    let userId: string;
    try {
      ({ userId } = await this.auth.completeLogin(code, verifier));
    } catch (err) {
      if (err instanceof OAuthCodeError) {
        incCounter('auth_logins_total', { result: 'invalid_code' });
        throw new BadRequestException('authorization code was rejected');
      }
      if (err instanceof GithubUnavailableError) {
        incCounter('auth_logins_total', { result: 'github_unavailable' });
        this.logger.warn(`GitHub unavailable during login (${err.message})`);
        this.redirectToLogin(res, 'github_unavailable');
        return;
      }
      throw err;
    }

    const session = await this.sessions.create(userId, req.headers['user-agent']);
    const maxAge = SESSION_TTL_SECONDS * 1000;
    res.cookie(SESSION_COOKIE, session.token, cookieOptions(this.config.NODE_ENV, { maxAge }));
    // Readable by the web app (not HttpOnly): it echoes the value in `X-CSRF-Token`.
    res.cookie(
      CSRF_COOKIE,
      newCsrfToken(),
      cookieOptions(this.config.NODE_ENV, { httpOnly: false, maxAge }),
    );
    incCounter('auth_logins_total', { result: 'success' });
    res.redirect(302, new URL('/', this.config.WEB_ORIGIN).toString());
  }

  @Post('logout')
  @HttpCode(204)
  async logout(
    @CurrentUser() user: RequestUser | undefined,
    @Res({ passthrough: true }) res: Response,
  ): Promise<void> {
    if (!user) throw new UnauthorizedException();
    await this.sessions.revoke(user.sessionId);
    clearCookie(res, SESSION_COOKIE, cookieOptions(this.config.NODE_ENV));
    clearCookie(res, CSRF_COOKIE, cookieOptions(this.config.NODE_ENV, { httpOnly: false }));
  }

  @Get('me')
  async me(@CurrentUser() user: RequestUser | undefined): Promise<{
    user: { id: string; login: string; display_name: string | null; avatar_url: string | null };
    organizations: { id: string; slug: string; display_name: string; role: string }[];
  }> {
    if (!user) throw new UnauthorizedException();
    const profile = await this.auth.profile(user.userId);
    if (!profile) throw new UnauthorizedException();
    const orgs = await this.auth.organizations(user.userId);
    return {
      user: {
        id: profile.id,
        login: profile.login,
        display_name: profile.displayName,
        avatar_url: profile.avatarUrl,
      },
      organizations: orgs.map((o) => ({
        id: o.organizationId,
        slug: o.slug,
        display_name: o.displayName,
        role: o.role,
      })),
    };
  }

  private requireGithub(): void {
    if (!this.config.GITHUB_ENABLED) throw new NotFoundException();
  }

  private redirectToLogin(res: Response, error: string): void {
    const url = new URL('/login', this.config.WEB_ORIGIN);
    url.searchParams.set('error', error);
    res.redirect(302, url.toString());
  }
}
