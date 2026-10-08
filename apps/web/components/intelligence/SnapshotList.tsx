import { Badge } from '@/components/ui/badge';
import type { SnapshotInfo } from '@/lib/api/pending';
import { formatDateTime, formatNumber, shortSha } from '@/lib/format';

/** Full and delta snapshots, newest first, with the delta chain length. */
export function SnapshotList({ snapshots }: { snapshots: SnapshotInfo[] }) {
  if (snapshots.length === 0) {
    return <p className="text-sm text-muted-foreground">No snapshots yet.</p>;
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm" aria-label="Snapshots">
        <thead className="text-xs text-muted-foreground uppercase">
          <tr>
            <th className="py-2 pr-4 font-medium">Kind</th>
            <th className="py-2 pr-4 font-medium">Commit</th>
            <th className="py-2 pr-4 font-medium">Branch</th>
            <th className="py-2 pr-4 text-right font-medium">Chain</th>
            <th className="py-2 pr-4 text-right font-medium">Nodes / edges</th>
            <th className="py-2 font-medium">Created</th>
          </tr>
        </thead>
        <tbody>
          {snapshots.map((s) => (
            <tr key={s.id} className="border-t">
              <td className="py-2 pr-4">
                <Badge variant={s.kind === 'full' ? 'secondary' : 'outline'}>{s.kind}</Badge>
              </td>
              <td className="py-2 pr-4 font-mono text-xs">{shortSha(s.commit_sha)}</td>
              <td className="py-2 pr-4 text-muted-foreground">{s.branch ?? '-'}</td>
              <td className="py-2 pr-4 text-right tabular-nums">{s.chain_length}</td>
              <td className="py-2 pr-4 text-right tabular-nums">
                {formatNumber(s.node_count)} / {formatNumber(s.edge_count)}
              </td>
              <td className="py-2 text-muted-foreground">{formatDateTime(s.created_at)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
