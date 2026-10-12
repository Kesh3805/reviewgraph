import { Inject, Injectable, Module, type OnModuleInit } from '@nestjs/common';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import { RepositoriesModule } from '../../repositories/repositories.module';
import { ProviderRegistry } from '../provider.registry';
import { BITBUCKET_API, createBitbucketApi } from './bitbucket-auth.service';
import { BitbucketEventNormalizer } from './bitbucket.normalizer';
import { BitbucketRepositoryProvider } from './bitbucket.provider';
import { BitbucketReviewPublisher } from './bitbucket.publisher';

/** Registers Bitbucket under `repositories.provider = 'bitbucket'`, only when it is enabled. */
@Injectable()
export class BitbucketProviderRegistration implements OnModuleInit {
  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    private readonly registry: ProviderRegistry,
    private readonly repository: BitbucketRepositoryProvider,
    private readonly publisher: BitbucketReviewPublisher,
  ) {}

  onModuleInit(): void {
    if (!this.config.BITBUCKET_ENABLED || this.registry.has('bitbucket')) return;
    this.registry.register('bitbucket', {
      repository: this.repository,
      publisher: this.publisher,
    });
  }
}

@Module({
  imports: [RepositoriesModule],
  providers: [
    {
      provide: BITBUCKET_API,
      inject: [APP_CONFIG],
      useFactory: (c: AppConfig) => createBitbucketApi(c),
    },
    BitbucketEventNormalizer,
    BitbucketRepositoryProvider,
    BitbucketReviewPublisher,
    BitbucketProviderRegistration,
  ],
  exports: [
    BITBUCKET_API,
    BitbucketEventNormalizer,
    BitbucketRepositoryProvider,
    BitbucketReviewPublisher,
  ],
})
export class BitbucketModule {}
