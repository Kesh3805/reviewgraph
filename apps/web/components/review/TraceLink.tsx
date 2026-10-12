import { ExternalLink } from 'lucide-react';

/** Public OpenObserve UI origin, e.g. `http://localhost:5080` (no trailing slash needed). */
export function openObserveUiUrl(): string | undefined {
  return process.env.NEXT_PUBLIC_OPENOBSERVE_UI_URL || undefined;
}

export function traceUrl(traceId: string, base = openObserveUiUrl()): string | null {
  if (!base) return null;
  return `${base.replace(/\/+$/, '')}/web/traces?trace_id=${encodeURIComponent(traceId)}`;
}

/** Deep link to the run's trace in OpenObserve; plain text when no UI URL is configured. */
export function TraceLink({ traceId, baseUrl }: { traceId: string | null; baseUrl?: string }) {
  if (!traceId) return <span className="text-muted-foreground">No trace</span>;
  const href = traceUrl(traceId, baseUrl ?? openObserveUiUrl());
  if (!href) return <span className="font-mono text-xs">{traceId}</span>;
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      className="inline-flex items-center gap-1 font-mono text-xs hover:underline"
    >
      {traceId}
      <ExternalLink className="size-3" aria-hidden />
      <span className="sr-only">Open trace in OpenObserve</span>
    </a>
  );
}
