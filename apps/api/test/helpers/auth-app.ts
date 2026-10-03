import { Controller, Delete, Get, Patch, Post } from '@nestjs/common';
import type { NestExpressApplication } from '@nestjs/platform-express';
import { GithubPermissionsMonitor } from '../../src/providers/github/permissions-monitor';
import { RequireRole } from '../../src/tenancy/roles.decorator';
import { createTestApp } from '../helpers';
import { generateAppKey } from './fake-github';
import type { FakeOAuth } from './fake-oauth';

const KEY = generateAppKey();

export const WEB_ORIGIN = 'http://localhost:3000';

/** A mutating route behind the session guard, standing in for any dashboard action. */
@Controller('csrf-probe')
export class MutationProbeController {
  @Post()
  post(): { ok: true } {
    return { ok: true };
  }

  @Patch()
  patch(): { ok: true } {
    return { ok: true };
  }

  @Delete()
  del(): { ok: true } {
    return { ok: true };
  }

  @Get()
  get(): { ok: true } {
    return { ok: true };
  }

  /** Needs a tenant: proves the session guard runs before the tenancy guard. */
  @Post('tenant')
  @RequireRole('viewer')
  tenant(): { ok: true } {
    return { ok: true };
  }
}

export function githubEnv(oauth: FakeOAuth): NodeJS.ProcessEnv {
  return {
    GITHUB_ENABLED: 'true',
    GITHUB_APP_ID: '12345',
    GITHUB_APP_PRIVATE_KEY: KEY.pem.replace(/\n/g, '\\n'),
    GITHUB_WEBHOOK_SECRET: 'whsec_auth_tests_0123456789',
    GITHUB_CLIENT_ID: 'client-id',
    GITHUB_CLIENT_SECRET: 'client-secret',
    GITHUB_OAUTH_URL: oauth.url,
    GITHUB_API_URL: oauth.url,
    WEB_ORIGIN,
  };
}

export function createAuthApp(
  oauth: FakeOAuth,
  options: Parameters<typeof createTestApp>[2] = {},
): Promise<NestExpressApplication> {
  return createTestApp(undefined, [MutationProbeController], {
    ...options,
    env: { ...githubEnv(oauth), ...options.env },
    configure: (builder) => {
      builder
        .overrideProvider(GithubPermissionsMonitor)
        .useValue({ enabled: false, status: () => 'unknown' });
      return options.configure ? options.configure(builder) : builder;
    },
  });
}
