import { Inject, Injectable, Logger, Optional } from '@nestjs/common';
import type { Redis } from 'ioredis';
import { sql } from 'kysely';
import { incCounter } from '../../common/metrics';
import { JOB_CANCELLER, type JobCanceller } from '../../common/job-canceller';
import { REDIS } from '../../common/redis.module';
import { DbService } from '../../db/db.module';
import type { Tx } from '../../db/tx';
import type {
  DeliveryContext,
  InstallationLifecycle,
  InstallationOutcome,
} from '../../webhooks/webhook.ports';
import type { InstallationEvent, InstallationRepository } from '../ports';
import { GithubPermissionsMonitor } from './permissions-monitor';

const PROVIDER = 'github';
/** Run states that still hold a worker; cancelled with a compare-and-set on `state`. */
const ACTIVE_RUN_STATES = [
  'RECEIVED',
  'INDEXING',
  'ANALYZING',
  'REVIEWING',
  'VERIFYING',
  'PUBLISHING',
] as const;

export type InstallationState = 'active' | 'suspended' | 'deleted';

/** The installation may not be used (suspended, deleted or unknown). Brokers answer 409. */
export class InstallationInactiveError extends Error {
  constructor(
    readonly installationId: string,
    readonly state: InstallationState | 'unknown',
  ) {
    super(`installation ${installationId} is not active (${state})`);
    this.name = 'InstallationInactiveError';
  }
}

/** Consulted before minting credentials or claiming work for an installation. */
export interface InstallationGate {
  state(installationId: string): Promise<InstallationState | null>;
  assertUsable(installationId: string): Promise<void>;
}
export const INSTALLATION_GATE = Symbol('INSTALLATION_GATE');

/**
 * GitHub installation lifecycle (GH-013): `installation.created|deleted|suspend|unsuspend|
 * new_permissions_accepted` and `installation_repositories.added|removed`. It runs inside the
 * webhook delivery transaction (GH-003), so a failure rolls the whole delivery back and GitHub's
 * retry starts clean. Every step is an upsert or an idempotent transition, so replays are safe.
 *
 * Revoked access stops work: repositories are disabled, active review runs move to `CANCELLED`
 * (compare-and-set on the state), queued jobs are cancelled through the `JobCanceller`, and
 * cached installation tokens are purged. Stages of in-flight work re-check `repositories.enabled`
 * (SUP-002) and the gate refuses new credentials.
 */
@Injectable()
export class InstallationService implements InstallationLifecycle, InstallationGate {
  private readonly logger = new Logger(InstallationService.name);

  constructor(
    private readonly dbs: DbService,
    @Inject(REDIS) private readonly redis: Redis,
    @Optional() @Inject(JOB_CANCELLER) private readonly jobs?: JobCanceller,
    @Optional() private readonly permissionsMonitor?: GithubPermissionsMonitor,
  ) {}

  async apply(ctx: DeliveryContext, event: InstallationEvent): Promise<InstallationOutcome> {
    incCounter('installation_events_total', { action: event.kind });
    const { trx } = ctx;
    const providerId = event.installationId;

    switch (event.kind) {
      case 'created':
      case 'repositories_added': {
        const org = await this.upsertInstallation(trx, event, event.kind === 'created');
        await this.enterTenant(trx, org);
        const installationRowId = await this.installationRowId(trx, providerId);
        await this.upsertRepositories(trx, org, installationRowId, event.added);
        return { applied: true, organizationId: org };
      }
      case 'repositories_removed': {
        const org = await this.lookupOrg(trx, providerId);
        if (!org) return { applied: false };
        await this.enterTenant(trx, org);
        const installationRowId = await this.installationRowId(trx, providerId);
        const ids = await this.disableRepositories(
          trx,
          installationRowId,
          'removed',
          event.removed.map((r) => r.providerRepoId),
        );
        await this.stopWork(trx, ids);
        return { applied: true, organizationId: org };
      }
      case 'deleted': {
        const org = await this.setState(trx, providerId, 'deleted', null);
        if (!org) return { applied: false };
        await this.enterTenant(trx, org);
        const installationRowId = await this.installationRowId(trx, providerId);
        const ids = await this.disableRepositories(trx, installationRowId, 'installation_deleted');
        await this.stopWork(trx, ids);
        await this.purgeTokens(providerId);
        // The retention purge of the installation data is scheduled by SEC-007 from
        // `provider_installations.deleted_at`.
        return { applied: true, organizationId: org };
      }
      case 'suspend':
      case 'unsuspend': {
        const org = await this.setState(
          trx,
          providerId,
          event.kind === 'suspend' ? 'suspended' : 'active',
          event.permissions,
        );
        if (!org) return { applied: false };
        if (event.kind === 'suspend') await this.purgeTokens(providerId);
        return { applied: true, organizationId: org };
      }
      case 'new_permissions_accepted': {
        const current = await this.stateOf(trx, providerId);
        if (!current) return { applied: false };
        const org = await this.setState(trx, providerId, current, event.permissions);
        return {
          applied: true,
          organizationId: org ?? undefined,
          // Re-runs the least-privilege check (GH-010) once the new permissions are recorded.
          afterCommit: () => {
            void this.permissionsMonitor?.verify().catch(() => undefined);
          },
        };
      }
    }
  }

  async state(installationId: string): Promise<InstallationState | null> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ state: InstallationState | null }>`
        select rg_installation_state(${PROVIDER}, ${installationId}::bigint) as state`.execute(trx),
    );
    return rows[0]?.state ?? null;
  }

  async assertUsable(installationId: string): Promise<void> {
    const state = await this.state(installationId);
    if (state !== 'active') throw new InstallationInactiveError(installationId, state ?? 'unknown');
  }

  // --- database steps (all inside the delivery transaction) ---

  private async upsertInstallation(
    trx: Tx,
    event: InstallationEvent,
    reactivate: boolean,
  ): Promise<string> {
    const { rows } = await sql<{ org: string }>`
      select rg_upsert_installation(
        ${PROVIDER}, ${event.installationId}::bigint, ${event.account.login},
        ${event.account.kind}, ${permissionsParam(event.permissions)}::jsonb, ${reactivate}) as org`.execute(
      trx,
    );
    return rows[0]!.org;
  }

  private async setState(
    trx: Tx,
    providerId: string,
    state: InstallationState,
    permissions: Record<string, string> | null,
  ): Promise<string | null> {
    const { rows } = await sql<{ org: string | null }>`
      select rg_set_installation_state(
        ${PROVIDER}, ${providerId}::bigint, ${state}, ${permissionsParam(permissions)}::jsonb) as org`.execute(
      trx,
    );
    return rows[0]?.org ?? null;
  }

  private async stateOf(trx: Tx, providerId: string): Promise<InstallationState | null> {
    const { rows } = await sql<{ state: InstallationState | null }>`
      select rg_installation_state(${PROVIDER}, ${providerId}::bigint) as state`.execute(trx);
    return rows[0]?.state ?? null;
  }

  private async lookupOrg(trx: Tx, providerId: string): Promise<string | null> {
    const { rows } = await sql<{ org: string | null }>`
      select rg_installation_org(${PROVIDER}, ${providerId}::bigint) as org`.execute(trx);
    return rows[0]?.org ?? null;
  }

  /** From here on the transaction is tenant scoped: RLS applies to every statement. */
  private async enterTenant(trx: Tx, organizationId: string): Promise<void> {
    await sql`select set_config('app.organization_id', ${organizationId}, true)`.execute(trx);
  }

  private async installationRowId(trx: Tx, providerId: string): Promise<string> {
    const row = await trx
      .selectFrom('provider_installations')
      .select('id')
      .where('provider', '=', PROVIDER)
      .where('provider_installation_id', '=', providerId)
      .executeTakeFirstOrThrow();
    return row.id;
  }

  /**
   * Inserts or re-activates repositories. The webhook carries only id, name and visibility, so a
   * new row gets the placeholder default branch `main` until the repository sync (GH-005) reads
   * the real one; an existing row keeps its branch.
   */
  private async upsertRepositories(
    trx: Tx,
    organizationId: string,
    installationRowId: string,
    repos: InstallationRepository[],
  ): Promise<void> {
    if (repos.length === 0) return;
    await trx
      .insertInto('repositories')
      .values(
        repos.map((r) => ({
          organization_id: organizationId,
          installation_id: installationRowId,
          provider: PROVIDER,
          provider_repo_id: r.providerRepoId,
          full_name: r.fullName,
          default_branch: 'main',
          visibility: r.isPrivate ? 'private' : 'public',
        })),
      )
      .onConflict((oc) =>
        oc.columns(['organization_id', 'provider', 'provider_repo_id']).doUpdateSet((eb) => ({
          installation_id: eb.ref('excluded.installation_id'),
          full_name: eb.ref('excluded.full_name'),
          visibility: eb.ref('excluded.visibility'),
          enabled: true,
          access_state: 'active',
        })),
      )
      .execute();
  }

  /** Disables repositories of the installation (all, or only the given provider ids). */
  private async disableRepositories(
    trx: Tx,
    installationRowId: string,
    accessState: 'removed' | 'installation_deleted',
    providerRepoIds?: string[],
  ): Promise<string[]> {
    if (providerRepoIds && providerRepoIds.length === 0) return [];
    let query = trx
      .updateTable('repositories')
      .set({ enabled: false, access_state: accessState })
      .where('installation_id', '=', installationRowId);
    if (providerRepoIds) query = query.where('provider_repo_id', 'in', providerRepoIds);
    const rows = await query.returning('id').execute();
    return rows.map((r) => r.id);
  }

  /** Cancels queued jobs and moves running review runs to CANCELLED (compare-and-set). */
  private async stopWork(trx: Tx, repositoryIds: string[]): Promise<void> {
    if (repositoryIds.length === 0) return;
    await this.jobs?.cancelQueuedForRepositories(trx, repositoryIds);
    await trx
      .updateTable('review_runs')
      .set({ state: 'CANCELLED', completed_at: sql<Date>`now()` })
      .where('repository_id', 'in', repositoryIds)
      .where('state', 'in', [...ACTIVE_RUN_STATES])
      .execute();
  }

  /** `DEL rg:gh:itok:{installation_id}:*` (SCAN, never KEYS) plus the mint lock. */
  private async purgeTokens(providerId: string): Promise<void> {
    try {
      const pattern = `rg:gh:itok:${providerId}:*`;
      let cursor = '0';
      do {
        const [next, keys] = await this.redis.scan(cursor, 'MATCH', pattern, 'COUNT', 100);
        cursor = next;
        if (keys.length) await this.redis.del(...keys);
      } while (cursor !== '0');
      await this.redis.del(`rg:gh:itok-lock:${providerId}`);
    } catch {
      // Tokens expire within the hour and the gate refuses new use meanwhile.
      this.logger.warn(
        `token purge failed installation=${providerId}; entries expire on their own`,
      );
    }
  }
}

function permissionsParam(permissions: Record<string, string> | null): string | null {
  return permissions && Object.keys(permissions).length > 0 ? JSON.stringify(permissions) : null;
}
