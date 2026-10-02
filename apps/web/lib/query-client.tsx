'use client';

import { MutationCache, QueryCache, QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useState, type ReactNode } from 'react';
import { ApiError } from './api-client';

export const STALE_TIME_MS = 30_000;

function redirectToLogin(error: unknown): void {
  if (error instanceof ApiError && error.isUnauthorized && typeof window !== 'undefined') {
    window.location.assign('/login');
  }
}

export function makeQueryClient(onError: (error: unknown) => void = redirectToLogin): QueryClient {
  return new QueryClient({
    // A 401 from any query or mutation sends the user to /login.
    queryCache: new QueryCache({ onError }),
    mutationCache: new MutationCache({ onError }),
    defaultOptions: {
      queries: {
        staleTime: STALE_TIME_MS,
        refetchOnWindowFocus: true,
        // Never retry client errors; retry transient failures twice.
        retry: (failureCount, error) =>
          !(error instanceof ApiError && error.status < 500) && failureCount < 2,
        // Surface 5xx to the nearest error boundary, which offers a retry button.
        throwOnError: (error) => error instanceof ApiError && error.status >= 500,
      },
    },
  });
}

export function QueryProvider({ children }: { children: ReactNode }) {
  const [client] = useState(() => makeQueryClient());
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
