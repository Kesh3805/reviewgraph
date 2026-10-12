import type { GraphStats as GraphStatsData } from '@/lib/api/pending';
import { formatNumber } from '@/lib/format';
import { ConfidenceHistogram } from './ConfidenceHistogram';

function CountTable({ title, counts }: { title: string; counts: Record<string, number> }) {
  const rows = Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  const total = rows.reduce((n, [, c]) => n + c, 0);
  return (
    <div>
      <h3 className="mb-1 text-xs font-medium text-muted-foreground uppercase">
        {title} ({formatNumber(total)})
      </h3>
      <table className="w-full text-sm" aria-label={title}>
        <tbody>
          {rows.map(([kind, count]) => (
            <tr key={kind} className="border-t">
              <td className="py-1 font-mono text-xs">{kind}</td>
              <td className="py-1 text-right tabular-nums">{formatNumber(count)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Nodes and edges by kind, unresolved references, parse failures, confidence, resolved_by. */
export function GraphStats({ stats }: { stats: GraphStatsData }) {
  return (
    <div className="space-y-6">
      <dl className="grid grid-cols-2 gap-4 text-sm sm:grid-cols-4">
        <div>
          <dt className="text-muted-foreground">Nodes</dt>
          <dd className="text-xl font-semibold tabular-nums" data-testid="stat-nodes">
            {formatNumber(Object.values(stats.nodes_by_kind).reduce((a, b) => a + b, 0))}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Edges</dt>
          <dd className="text-xl font-semibold tabular-nums" data-testid="stat-edges">
            {formatNumber(Object.values(stats.edges_by_kind).reduce((a, b) => a + b, 0))}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Unresolved references</dt>
          <dd className="text-xl font-semibold tabular-nums" data-testid="stat-unresolved">
            {formatNumber(stats.unresolved_references)}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Parse failures</dt>
          <dd className="text-xl font-semibold tabular-nums" data-testid="stat-parse-failures">
            {formatNumber(stats.parse_failures)}
          </dd>
        </div>
      </dl>
      <div className="grid gap-6 md:grid-cols-3">
        <CountTable title="Nodes by kind" counts={stats.nodes_by_kind} />
        <CountTable title="Edges by kind" counts={stats.edges_by_kind} />
        <CountTable title="Resolved by" counts={stats.resolved_by} />
      </div>
      <div>
        <h3 className="mb-1 text-xs font-medium text-muted-foreground uppercase">
          Edge confidence
        </h3>
        <ConfidenceHistogram counts={stats.edge_confidence_histogram} />
      </div>
    </div>
  );
}
