import { Module } from '@nestjs/common';
import { NullProfileReader, PROFILE_READER } from './profile.port';
import { PgRepositorySettings } from './pg-repository-settings';
import { RepositoriesController } from './repositories.controller';
import { RepositoriesService } from './repositories.service';
import { REPOSITORY_SETTINGS } from './repository-settings.port';
import { RepositorySyncService } from './sync.service';

@Module({
  controllers: [RepositoriesController],
  providers: [
    RepositoriesService,
    // PROF-001 replaces the null reader once profiles are computed.
    { provide: PROFILE_READER, useClass: NullProfileReader },
    // The review guards read these settings during webhook normalization.
    { provide: REPOSITORY_SETTINGS, useClass: PgRepositorySettings },
    RepositorySyncService,
  ],
  exports: [RepositoriesService, REPOSITORY_SETTINGS, RepositorySyncService],
})
export class RepositoriesModule {}
