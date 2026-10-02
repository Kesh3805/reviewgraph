import Link from 'next/link';
import type { ReactNode } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';

export type CardState = 'loading' | 'error' | 'unavailable' | 'ready';

/** A metric card. Every card links to the list the metric was computed from. */
export function StatCard({
  title,
  href,
  state,
  onRetry,
  children,
}: {
  title: string;
  href: string;
  state: CardState;
  onRetry?: () => void;
  children?: ReactNode;
}) {
  return (
    <Card data-testid={`card-${title}`}>
      <CardHeader>
        <CardTitle className="text-sm font-medium text-muted-foreground">
          <Link href={href} className="hover:underline">
            {title}
          </Link>
        </CardTitle>
      </CardHeader>
      <CardContent>
        {state === 'loading' && (
          <div
            role="status"
            aria-label={`Loading ${title}`}
            className="h-8 animate-pulse rounded bg-muted"
          />
        )}
        {state === 'error' && (
          <div role="alert" className="flex items-center justify-between gap-2 text-sm">
            <span className="text-destructive">Could not load this metric.</span>
            {onRetry && (
              <Button size="sm" variant="outline" onClick={onRetry}>
                Retry
              </Button>
            )}
          </div>
        )}
        {state === 'unavailable' && (
          <p className="text-sm text-muted-foreground">Not available yet.</p>
        )}
        {state === 'ready' && children}
      </CardContent>
    </Card>
  );
}
