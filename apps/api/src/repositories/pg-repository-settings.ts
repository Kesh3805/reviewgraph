import { Injectable } from '@nestjs/common';
import { sql } from 'kysely';
import { DbService } from '../db/db.module';
import type { RepoRef } from '../providers/ports';
import {
  DEFAULT_REPOSITORY_SETTINGS,
  type RepositorySettings,
  type RepositorySettingsPort,
} from './repository-settings.port';

/**
 * Review settings from `repository_settings` for webhook normalization, which runs before any
 * tenant is known: the lookup goes through the `rg_repository_settings` SECURITY DEFINER
 * function. A repository that lost access (removed from the installation, installation deleted
 * or suspended) is reported disabled; one that is not synced yet gets the defaults.
 */
@Injectable()
export class PgRepositorySettings implements RepositorySettingsPort {
  constructor(private readonly dbs: DbService) {}

  async getSettings(repo: RepoRef): Promise<RepositorySettings> {
    if (!/^\d+$/.test(repo.installationId)) return { ...DEFAULT_REPOSITORY_SETTINGS };
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{
        enabled: boolean;
        target_branches: string[];
        skip_drafts: boolean;
        skip_bots: boolean;
      }>`select * from rg_repository_settings(
          ${repo.provider}, ${repo.installationId}::bigint, ${`${repo.owner}/${repo.name}`})`.execute(
        trx,
      ),
    );
    const row = rows[0];
    if (!row) return { ...DEFAULT_REPOSITORY_SETTINGS };
    return {
      enabled: row.enabled,
      targetBranches: row.target_branches,
      skipDrafts: row.skip_drafts,
      skipBots: row.skip_bots,
    };
  }
}
