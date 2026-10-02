import { Injectable, Module } from '@nestjs/common';
import type { RepoRef } from '../providers/ports';

/**
 * Per-repository review settings (`repository_settings`, API-008). The legacy guard options
 * from `github.rs:305-347` live here.
 */
export interface RepositorySettings {
  enabled: boolean;
  /** Base-branch patterns; empty means every branch. A trailing `*` is a prefix match. */
  targetBranches: string[];
  skipDrafts: boolean;
  skipBots: boolean;
}

export const DEFAULT_REPOSITORY_SETTINGS: RepositorySettings = {
  enabled: true,
  targetBranches: [],
  skipDrafts: true,
  skipBots: true,
};

export interface RepositorySettingsPort {
  getSettings(repo: RepoRef): Promise<RepositorySettings>;
}
export const REPOSITORY_SETTINGS = Symbol('REPOSITORY_SETTINGS');

/** Defaults for every repository until API-008 persists real settings. */
@Injectable()
export class DefaultRepositorySettings implements RepositorySettingsPort {
  getSettings(): Promise<RepositorySettings> {
    return Promise.resolve({ ...DEFAULT_REPOSITORY_SETTINGS });
  }
}

@Module({
  providers: [{ provide: REPOSITORY_SETTINGS, useClass: DefaultRepositorySettings }],
  exports: [REPOSITORY_SETTINGS],
})
export class RepositoriesModule {}
