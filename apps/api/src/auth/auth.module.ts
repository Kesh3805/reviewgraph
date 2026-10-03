import { Global, Module } from '@nestjs/common';
import { APP_GUARD } from '@nestjs/core';
import { AuthController } from './auth.controller';
import { AuthService } from './auth.service';
import { CsrfGuard } from './csrf.guard';
import { GithubOAuthClient } from './github-oauth.client';
import { SessionGuard } from './session.guard';
import { SessionService } from './session.service';

/**
 * Sessions, GitHub OAuth login and CSRF (API-004). The guards are global and run in this order:
 * session (401), CSRF (403), then the tenancy guard (TenancyModule must be imported after this).
 */
@Global()
@Module({
  controllers: [AuthController],
  providers: [
    AuthService,
    GithubOAuthClient,
    SessionService,
    { provide: APP_GUARD, useClass: SessionGuard },
    { provide: APP_GUARD, useClass: CsrfGuard },
  ],
  exports: [SessionService],
})
export class AuthModule {}
