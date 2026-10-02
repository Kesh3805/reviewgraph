import { Global, Injectable, Module } from '@nestjs/common';
import {
  PROVIDER_RESOLVER,
  type ProviderKind,
  type ProviderResolver,
  type RepositoryProvider,
  type ReviewPublisher,
} from './ports';

/** Injection tokens, one per provider kind, for modules that want a specific implementation. */
export const repositoryProviderToken = (kind: ProviderKind): symbol =>
  Symbol.for(`rg:provider:repository:${kind}`);
export const reviewPublisherToken = (kind: ProviderKind): symbol =>
  Symbol.for(`rg:provider:publisher:${kind}`);

export class UnknownProviderError extends Error {
  constructor(readonly kind: string) {
    super(`no provider registered for kind "${kind}"`);
    this.name = 'UnknownProviderError';
  }
}

/** Registry keyed by `repositories.provider`. Provider modules register themselves at init. */
@Injectable()
export class ProviderRegistry implements ProviderResolver {
  private readonly repositories = new Map<ProviderKind, RepositoryProvider>();
  private readonly publishers = new Map<ProviderKind, ReviewPublisher>();

  register(
    kind: ProviderKind,
    impl: { repository: RepositoryProvider; publisher: ReviewPublisher },
  ): void {
    if (this.repositories.has(kind)) {
      throw new Error(`provider "${kind}" is already registered`);
    }
    this.repositories.set(kind, impl.repository);
    this.publishers.set(kind, impl.publisher);
  }

  has(kind: ProviderKind): boolean {
    return this.repositories.has(kind);
  }

  kinds(): ProviderKind[] {
    return [...this.repositories.keys()];
  }

  repository(kind: ProviderKind): RepositoryProvider {
    const provider = this.repositories.get(kind);
    if (!provider) throw new UnknownProviderError(kind);
    return provider;
  }

  publisher(kind: ProviderKind): ReviewPublisher {
    const publisher = this.publishers.get(kind);
    if (!publisher) throw new UnknownProviderError(kind);
    return publisher;
  }
}

@Global()
@Module({
  providers: [ProviderRegistry, { provide: PROVIDER_RESOLVER, useExisting: ProviderRegistry }],
  exports: [ProviderRegistry, PROVIDER_RESOLVER],
})
export class ProvidersModule {}
