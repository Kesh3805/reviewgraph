import { Inject, Injectable, OnModuleDestroy, OnModuleInit, Optional } from '@nestjs/common';
import {
  GITHUB_PERMISSIONS_STATUS,
  type GithubPermissionsStatus,
  type PermissionsStatus,
} from '../providers/github/permissions-monitor';
import { HEALTH_PROBES, type HealthProbes } from './health.probes';

export type CheckStatus = 'up' | 'down';

export interface ReadyReport {
  status: 'ok' | 'unavailable';
  checks: {
    pg: CheckStatus;
    redis: CheckStatus;
    engine: CheckStatus;
    /** Present only when GitHub is enabled; `invalid` makes readiness fail (GH-010). */
    github_permissions?: PermissionsStatus;
  };
}

export const READY_CACHE_MS = 2000;
/** Liveness fails when the event loop has not ticked for longer than this. */
export const LOOP_BLOCKED_MS = 5000;
const TICK_MS = 500;

@Injectable()
export class HealthService implements OnModuleInit, OnModuleDestroy {
  private cached?: { at: number; report: ReadyReport };
  private inflight?: Promise<ReadyReport>;
  private lastTick = Date.now();
  private timer?: NodeJS.Timeout;

  constructor(
    @Inject(HEALTH_PROBES) private readonly probes: HealthProbes,
    @Optional()
    @Inject(GITHUB_PERMISSIONS_STATUS)
    private readonly githubPermissions?: GithubPermissionsStatus & { enabled?: boolean },
  ) {}

  onModuleInit(): void {
    this.lastTick = Date.now();
    this.timer = setInterval(() => {
      this.lastTick = Date.now();
    }, TICK_MS);
    this.timer.unref();
  }

  onModuleDestroy(): void {
    if (this.timer) clearInterval(this.timer);
  }

  /** True while the event loop is making progress. */
  isLive(now = Date.now()): boolean {
    return now - this.lastTick <= LOOP_BLOCKED_MS;
  }

  /** Readiness result, cached for 2 s so probe storms do not hammer dependencies. */
  ready(now = Date.now()): Promise<ReadyReport> {
    if (this.cached && now - this.cached.at < READY_CACHE_MS) {
      return Promise.resolve(this.cached.report);
    }
    this.inflight ??= this.check()
      .then((report) => {
        this.cached = { at: Date.now(), report };
        return report;
      })
      .finally(() => {
        this.inflight = undefined;
      });
    return this.inflight;
  }

  private async check(): Promise<ReadyReport> {
    const [pg, redis, engine] = await Promise.all([
      settle(this.probes.pg()),
      settle(this.probes.redis()),
      settle(this.probes.engine()),
    ]);
    const checks: ReadyReport['checks'] = { pg, redis, engine };
    if (this.githubPermissions?.enabled)
      checks.github_permissions = this.githubPermissions.status();
    const ok =
      [pg, redis, engine].every((c) => c === 'up') && checks.github_permissions !== 'invalid';
    return { status: ok ? 'ok' : 'unavailable', checks };
  }
}

async function settle(probe: Promise<void>): Promise<CheckStatus> {
  try {
    await probe;
    return 'up';
  } catch {
    return 'down';
  }
}
