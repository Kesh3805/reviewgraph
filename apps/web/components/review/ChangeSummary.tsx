import type { ChangeSummary as ChangeSummaryData } from '@/lib/api/pending';

const FILE_STATUSES = ['added', 'modified', 'deleted', 'renamed'] as const;

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1">
      <h3 className="text-xs font-medium text-muted-foreground uppercase">{title}</h3>
      {children}
    </div>
  );
}

function None() {
  return <p className="text-sm text-muted-foreground">None</p>;
}

/** The change model: files by status, behavioral symbols, API contracts, dependencies, schemas. */
export function ChangeSummary({ change }: { change: ChangeSummaryData | null }) {
  if (!change) {
    return <p className="text-sm text-muted-foreground">The change model is not available.</p>;
  }
  return (
    <div className="grid gap-4 text-sm md:grid-cols-2">
      <Section title="Files by status">
        <ul className="flex flex-wrap gap-3">
          {FILE_STATUSES.map((status) => (
            <li key={status}>
              <span className="capitalize">{status}</span>{' '}
              <span className="font-semibold tabular-nums">
                {change.files.filter((f) => f.status === status).length}
              </span>
            </li>
          ))}
        </ul>
      </Section>
      <Section title="Behavioral symbols">
        {change.behavioral_symbols.length === 0 ? (
          <None />
        ) : (
          <ul className="space-y-0.5">
            {change.behavioral_symbols.map((s) => (
              <li key={s.key} className="flex justify-between gap-2">
                <span className="truncate font-mono text-xs">{s.name}</span>
                <span className="text-muted-foreground">
                  {s.kind} · {s.change.replace('_', ' ')}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Section>
      <Section title="API contracts">
        {change.api_contracts.length === 0 ? (
          <None />
        ) : (
          <ul>
            {change.api_contracts.map((c) => (
              <li key={c.name}>
                <span className="font-mono text-xs">{c.name}</span>{' '}
                <span className="text-muted-foreground">{c.change}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>
      <Section title="Dependencies">
        {change.dependencies.length === 0 ? (
          <None />
        ) : (
          <ul>
            {change.dependencies.map((d) => (
              <li key={d.name}>
                <span className="font-mono text-xs">{d.name}</span>{' '}
                <span className="text-muted-foreground">
                  {d.from ?? 'none'} → {d.to ?? 'removed'}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Section>
      <Section title="Schemas">
        {change.schemas.length === 0 ? (
          <None />
        ) : (
          <ul>
            {change.schemas.map((s) => (
              <li key={s.name}>
                <span className="font-mono text-xs">{s.name}</span>{' '}
                <span className="text-muted-foreground">{s.change}</span>
              </li>
            ))}
          </ul>
        )}
      </Section>
    </div>
  );
}
