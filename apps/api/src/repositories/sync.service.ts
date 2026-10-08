import { Inject, Injectable, Logger } from '@nestjs/common';
import { sql } from 'kysely';
import { incCounter } from '../common/metrics';
import { DbService } from '../db/db.module';
import type { Tx } from '../db/tx';
import {
  PROVIDER_RESOLVER,
  ProviderError,
  type ProviderResolver,
  type RepoRef,
} from '../providers/ports';

/** A repository row with the provider coordinates the orchestrator needs. */
export interface SyncedRepository {
  id: string;
  organizationId: string;
  providerRepoId: string;
  fullName: string;
  defaultBranch: string;
  enabled: boolean;
  accessState: string;
  ref: RepoRef;
}

export type RepositorySyncResult =
  | { status: 'synced'; repository: SyncedRepository }
  /** The installation is unknown here (no organization yet). */
  | { status: 'unknown_installation' }
  /** The provider answered 404: the row (if any) is marked `access_lost`. */
  | { status: 'access_lost'; repositoryId?: string };

/**
 * Keeps `repositories` current from provider reads (GH-005). Installation events create rows
 * with a placeholder default branch; the first sync (or an explicit refresh) reads the
 * authoritative repository. A 404 means the repository was removed or access was revoked: the
 * row is disabled with `access_state = 'access_lost'` and the event is ignored.
 */
@Injectable()
export class RepositorySyncService {
  private readonly logger = new Logger(RepositorySyncService.name);

  constructor(
    private readonly dbs: DbService,
    @Inject(PROVIDER_RESOLVER) private readonly providers: ProviderResolver,
  ) {}

  /** Organization owning a provider installation, or null when it is unknown. */
  async organizationOf(ref: RepoRef): Promise<string | null> {
    if (!/^\d+$/.test(ref.installationId)) return null;
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ org: string | null }>`
        select rg_installation_org(${ref.provider}, ${ref.installationId}::bigint) as org`.execute(
        trx,
      ),
    );
    return rows[0]?.org ?? null;
  }

  /**
   * Returns the repository row of an event, creating or refreshing it from the provider when it
   * is missing or `refresh` is set.
   */
  async upsertFromEvent(
    ref: RepoRef,
    opts: { refresh?: boolean } = {},
  ): Promise<RepositorySyncResult> {
    const org = await this.organizationOf(ref);
    if (!org) return { status: 'unknown_installation' };

    const existing = await this.dbs.withTx(org, (trx) => this.find(trx, ref));
    if (existing && !opts.refresh) return { status: 'synced', repository: existing };

    let remote;
    try {
      remote = await this.providers.repository(ref.provider).getRepository(ref);
    } catch (err) {
      if (err instanceof ProviderError && err.kind === 'not_found') {
        if (existing) await this.markAccessLost(org, existing.id);
        incCounter('repository_sync_total', { outcome: 'access_lost' });
        return { status: 'access_lost', repositoryId: existing?.id };
      }
      throw err;
    }

    const repository = await this.dbs.withTx(org, async (trx) => {
      const installation = await trx
        .selectFrom('provider_installations')
        .select('id')
        .where('provider', '=', ref.provider)
        .where('provider_installation_id', '=', ref.installationId)
        .executeTakeFirstOrThrow();
      await trx
        .insertInto('repositories')
        .values({
          organization_id: org,
          installation_id: installation.id,
          provider: ref.provider,
          provider_repo_id: remote.providerRepoId,
          full_name: `${ref.owner}/${ref.name}`,
          default_branch: remote.defaultBranch,
          visibility: remote.isPrivate ? 'private' : 'public',
          archived: remote.archived,
        })
        .onConflict((oc) =>
          oc.columns(['organization_id', 'provider', 'provider_repo_id']).doUpdateSet((eb) => ({
            installation_id: eb.ref('excluded.installation_id'),
            full_name: eb.ref('excluded.full_name'),
            default_branch: eb.ref('excluded.default_branch'),
            visibility: eb.ref('excluded.visibility'),
            archived: eb.ref('excluded.archived'),
          })),
        )
        .execute();
      return this.find(trx, ref);
    });
    if (!repository) throw new Error('repository row missing after upsert');
    incCounter('repository_sync_total', { outcome: 'synced' });
    return { status: 'synced', repository };
  }

  /** Disables a repository whose provider access is gone. Idempotent. */
  async markAccessLost(organizationId: string, repositoryId: string): Promise<void> {
    await this.dbs.withTx(organizationId, (trx) =>
      trx
        .updateTable('repositories')
        .set({ enabled: false, access_state: 'access_lost' })
        .where('id', '=', repositoryId)
        .execute(),
    );
    this.logger.warn(`repository access lost repository=${repositoryId}`);
  }

  private async find(trx: Tx, ref: RepoRef): Promise<SyncedRepository | null> {
    const row = await trx
      .selectFrom('repositories as r')
      .innerJoin('provider_installations as i', 'i.id', 'r.installation_id')
      .select([
        'r.id',
        'r.organization_id',
        'r.provider_repo_id',
        'r.full_name',
        'r.default_branch',
        'r.enabled',
        'r.access_state',
      ])
      .where('i.provider', '=', ref.provider)
      .where('i.provider_installation_id', '=', ref.installationId)
      .where(sql<boolean>`lower(r.full_name) = lower(${`${ref.owner}/${ref.name}`})`)
      .executeTakeFirst();
    if (!row) return null;
    return {
      id: row.id,
      organizationId: row.organization_id,
      providerRepoId: row.provider_repo_id,
      fullName: row.full_name,
      defaultBranch: row.default_branch,
      enabled: row.enabled,
      accessState: row.access_state,
      ref,
    };
  }
}
