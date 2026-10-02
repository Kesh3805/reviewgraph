import Link from 'next/link';
import { isTerminal, type ActiveRun } from '@/lib/dashboard';

export function formatAge(startedAt: string, now = Date.now()): string {
  const seconds = Math.max(0, Math.floor((now - new Date(startedAt).getTime()) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours}h` : `${Math.floor(hours / 24)}d`;
}

export function ActiveRunsTable({ runs, now }: { runs: ActiveRun[]; now?: number }) {
  if (runs.length === 0) {
    return <p className="text-sm text-muted-foreground">No active review runs.</p>;
  }
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead className="text-xs text-muted-foreground uppercase">
          <tr>
            <th className="py-2 pr-4 font-medium">Pull request</th>
            <th className="py-2 pr-4 font-medium">State</th>
            <th className="py-2 pr-4 font-medium">Stage</th>
            <th className="py-2 font-medium">Age</th>
          </tr>
        </thead>
        <tbody>
          {runs.map((run) => (
            <tr key={run.id} className="border-t">
              <td className="py-2 pr-4">
                <Link
                  href={`/pull-requests?run=${encodeURIComponent(run.id)}`}
                  className="hover:underline"
                >
                  {run.repository}#{run.pull_request_number}
                </Link>
              </td>
              <td className="py-2 pr-4">
                <span
                  className={
                    isTerminal(run.state) ? 'text-muted-foreground' : 'font-medium text-foreground'
                  }
                >
                  {run.state}
                </span>
              </td>
              <td className="py-2 pr-4 text-muted-foreground">{run.stage ?? '-'}</td>
              <td className="py-2 text-muted-foreground">{formatAge(run.started_at, now)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
