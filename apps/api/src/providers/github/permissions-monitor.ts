import { Inject, Injectable, Logger, OnApplicationBootstrap, Optional } from '@nestjs/common';
import { setGauge } from '../../common/metrics';
import type { GithubAppAuth } from './app-auth.service';
import { diffPermissions, permissionsMatch } from './app-manifest';
import { GITHUB_APP_AUTH } from './github.tokens';

export type PermissionsStatus = 'valid' | 'invalid' | 'unknown';

/** What readiness needs to know; implemented by the monitor, absent when GitHub is disabled. */
export interface GithubPermissionsStatus {
  status(): PermissionsStatus;
}
export const GITHUB_PERMISSIONS_STATUS = Symbol('GITHUB_PERMISSIONS_STATUS');

/**
 * Boot-time least-privilege check (GH-010): `GET /app` must report exactly the manifest
 * permissions. A mismatch (for example `contents: write`) logs a critical message, sets
 * `github_app_permissions_valid` to 0 and keeps publishing disabled; the publish consumer asks
 * `canPublish()` before it starts. If GitHub cannot be reached the status stays `unknown`, which
 * also keeps publishing off until a later `verify()` succeeds.
 */
@Injectable()
export class GithubPermissionsMonitor implements GithubPermissionsStatus, OnApplicationBootstrap {
  private readonly logger = new Logger(GithubPermissionsMonitor.name);
  private current: PermissionsStatus = 'unknown';

  constructor(@Optional() @Inject(GITHUB_APP_AUTH) private readonly auth: GithubAppAuth | null) {}

  get enabled(): boolean {
    return this.auth !== null && this.auth !== undefined;
  }

  status(): PermissionsStatus {
    return this.current;
  }

  canPublish(): boolean {
    return this.current === 'valid';
  }

  async onApplicationBootstrap(): Promise<void> {
    if (this.enabled) await this.verify();
  }

  async verify(): Promise<PermissionsStatus> {
    if (!this.auth) return this.current;
    try {
      const actual = await this.auth.getAppPermissions();
      const diff = diffPermissions(actual);
      if (permissionsMatch(diff)) {
        this.current = 'valid';
      } else {
        this.current = 'invalid';
        this.logger.error(
          `CRITICAL: GitHub App permissions differ from the manifest; publishing disabled. ` +
            `extra=[${diff.extra.join(',')}] missing=[${diff.missing.join(',')}] ` +
            `mismatched=[${diff.mismatched.join(',')}]`,
        );
      }
    } catch (err) {
      this.current = 'unknown';
      this.logger.error(
        `could not verify GitHub App permissions; publishing disabled (${err instanceof Error ? err.name : 'error'})`,
      );
    }
    setGauge('github_app_permissions_valid', this.current === 'valid' ? 1 : 0);
    return this.current;
  }
}
