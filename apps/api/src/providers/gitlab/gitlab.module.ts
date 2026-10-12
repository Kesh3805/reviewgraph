import { Inject, Injectable, Module, type OnModuleInit } from '@nestjs/common';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import { RepositoriesModule } from '../../repositories/repositories.module';
import { ProviderRegistry } from '../provider.registry';
import { GITLAB_API, createGitlabApi } from './gitlab-auth.service';
import { GitlabEventNormalizer } from './gitlab.normalizer';
import { GitlabRepositoryProvider } from './gitlab.provider';
import { GitlabReviewPublisher } from './gitlab.publisher';

/** Registers GitLab under `repositories.provider = 'gitlab'`, only when it is enabled. */
@Injectable()
export class GitlabProviderRegistration implements OnModuleInit {
  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    private readonly registry: ProviderRegistry,
    private readonly repository: GitlabRepositoryProvider,
    private readonly publisher: GitlabReviewPublisher,
  ) {}

  onModuleInit(): void {
    if (!this.config.GITLAB_ENABLED || this.registry.has('gitlab')) return;
    this.registry.register('gitlab', { repository: this.repository, publisher: this.publisher });
  }
}

@Module({
  imports: [RepositoriesModule],
  providers: [
    { provide: GITLAB_API, inject: [APP_CONFIG], useFactory: (c: AppConfig) => createGitlabApi(c) },
    GitlabEventNormalizer,
    GitlabRepositoryProvider,
    GitlabReviewPublisher,
    GitlabProviderRegistration,
  ],
  exports: [GITLAB_API, GitlabEventNormalizer, GitlabRepositoryProvider, GitlabReviewPublisher],
})
export class GitlabModule {}
