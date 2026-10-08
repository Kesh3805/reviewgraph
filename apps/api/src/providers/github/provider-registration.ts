import { Injectable, type OnModuleInit } from '@nestjs/common';
import { ProviderRegistry } from '../provider.registry';
import { GithubRepositoryProvider } from './repository-provider';
import { GithubReviewPublisher } from './review-publisher';

/** Registers the GitHub provider under `repositories.provider = 'github'`. */
@Injectable()
export class GithubProviderRegistration implements OnModuleInit {
  constructor(
    private readonly registry: ProviderRegistry,
    private readonly repository: GithubRepositoryProvider,
    private readonly publisher: GithubReviewPublisher,
  ) {}

  onModuleInit(): void {
    if (this.registry.has('github')) return;
    this.registry.register('github', { repository: this.repository, publisher: this.publisher });
  }
}
