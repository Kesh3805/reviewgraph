import type { ReactNode } from 'react';
import { ApiError } from '@/lib/api-client';
import { Button } from './button';

/** Skeleton placeholder announced to assistive technology. */
export function Loading({ label, className }: { label: string; className?: string }) {
  return (
    <div
      role="status"
      aria-label={label}
      className={className ?? 'h-24 animate-pulse rounded-md bg-muted'}
    />
  );
}

/** Human message for a failed request; never includes stack traces. */
export function errorMessage(error: unknown, fallback = 'Something went wrong.'): string {
  if (error instanceof ApiError) {
    if (error.status === 403) return 'You do not have access to this.';
    if (error.status === 404) return 'Not found.';
    if (error.status === 503) return 'The service is temporarily unavailable.';
    return error.message || fallback;
  }
  return fallback;
}

export function ErrorState({
  error,
  title = 'Could not load this.',
  onRetry,
}: {
  error?: unknown;
  title?: string;
  onRetry?: () => void;
}) {
  return (
    <div
      role="alert"
      className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-destructive/30 p-3 text-sm"
    >
      <span>
        <span className="font-medium text-destructive">{title}</span>{' '}
        {error !== undefined && (
          <span className="text-muted-foreground">{errorMessage(error, '')}</span>
        )}
      </span>
      {onRetry && (
        <Button size="sm" variant="outline" onClick={onRetry}>
          Retry
        </Button>
      )}
    </div>
  );
}

export function EmptyState({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="rounded-md border border-dashed p-6 text-center text-sm">
      <p className="font-medium">{title}</p>
      {children && <div className="mt-1 text-muted-foreground">{children}</div>}
    </div>
  );
}
