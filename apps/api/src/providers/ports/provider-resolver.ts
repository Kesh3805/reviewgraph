import type { RepositoryProvider } from './repository-provider.port';
import type { ReviewPublisher } from './review-publisher.port';
import type { ProviderKind } from './types';

export const PROVIDER_RESOLVER = Symbol('PROVIDER_RESOLVER');

/**
 * How domain modules obtain a provider by `repositories.provider`. They inject this port, never
 * a concrete provider.
 */
export interface ProviderResolver {
  repository(kind: ProviderKind): RepositoryProvider;
  publisher(kind: ProviderKind): ReviewPublisher;
}
