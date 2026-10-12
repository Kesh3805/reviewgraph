'use client';

import { useQuery } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { ConfidenceHistogram } from '@/components/intelligence/ConfidenceHistogram';
import { Button } from '@/components/ui/button';
import { Loading } from '@/components/ui/states';
import { symbolQuery } from '@/lib/queries';
import { isGraphUnavailable } from './SymbolSearch';

function Counts({ title, counts }: { title: string; counts: Record<string, number> }) {
  const rows = Object.entries(counts).sort((a, b) => b[1] - a[1]);
  return (
    <div>
      <h4 className="text-xs font-medium text-muted-foreground uppercase">{title}</h4>
      {rows.length === 0 ? (
        <p className="text-xs text-muted-foreground">None</p>
      ) : (
        <ul className="text-xs" aria-label={title}>
          {rows.map(([kind, n]) => (
            <li key={kind} className="flex justify-between gap-2">
              <span className="font-mono">{kind}</span>
              <span className="tabular-nums">{n}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** The CLI-007 symbol fields: kind, id, key, location, signature, facts, lineage, edges. */
export function SymbolPanel({
  repoId,
  symbolKey,
  snapshot,
  actions,
}: {
  repoId: string;
  symbolKey: string;
  snapshot?: string;
  actions?: ReactNode;
}) {
  const query = useQuery(symbolQuery(repoId, symbolKey, snapshot));
  if (query.isPending) return <Loading label="Loading symbol" />;
  if (query.isError) {
    return (
      <div role="alert" className="flex items-center justify-between rounded-md border p-3 text-sm">
        <span className="text-destructive">
          {isGraphUnavailable(query.error) ? 'Graph service unavailable.' : 'Symbol not found.'}
        </span>
        <Button size="sm" variant="outline" onClick={() => void query.refetch()}>
          Retry
        </Button>
      </div>
    );
  }
  const s = query.data;
  return (
    <section aria-label="Symbol" className="space-y-3 rounded-md border p-4 text-sm">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <h2 className="font-mono text-base font-semibold">{s.qualified_name}</h2>
          <p className="text-muted-foreground">
            {s.kind}
            {s.visibility && ` · ${s.visibility}`} · {s.path}:{s.start_line}-{s.end_line}
          </p>
        </div>
        {actions}
      </div>
      {s.signature && (
        <pre className="overflow-x-auto rounded bg-muted/50 p-2 font-mono text-xs whitespace-pre-wrap">
          {s.signature}
        </pre>
      )}
      <dl className="grid grid-cols-[6rem_1fr] gap-y-1 text-xs">
        <dt className="text-muted-foreground">Key</dt>
        <dd className="font-mono break-all">{s.key}</dd>
        <dt className="text-muted-foreground">Id</dt>
        <dd className="font-mono break-all">{s.id}</dd>
        <dt className="text-muted-foreground">Snapshot</dt>
        <dd className="font-mono">{s.snapshot_id}</dd>
      </dl>
      {s.framework_facts.length > 0 && (
        <div>
          <h4 className="text-xs font-medium text-muted-foreground uppercase">Framework facts</h4>
          <ul className="text-xs">
            {s.framework_facts.map((f) => (
              <li key={`${f.name}=${f.value}`}>
                <span className="font-mono">{f.name}</span>: {f.value}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="grid grid-cols-2 gap-4">
        <Counts title="Incoming edges" counts={s.edges_in} />
        <Counts title="Outgoing edges" counts={s.edges_out} />
      </div>
      {s.lineage.length > 0 && (
        <div>
          <h4 className="text-xs font-medium text-muted-foreground uppercase">Lineage</h4>
          <ul className="text-xs">
            {s.lineage.map((l) => (
              <li key={`${l.snapshot_id}:${l.key}`}>
                <span className="font-mono">{l.key}</span> · {l.change} · {l.snapshot_id}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div>
        <h4 className="text-xs font-medium text-muted-foreground uppercase">Edge confidence</h4>
        <ConfidenceHistogram counts={s.confidence_histogram} />
      </div>
    </section>
  );
}
