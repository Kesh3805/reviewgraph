import type { RepositoryProfile } from '@/lib/api/pending';

type Architecture = RepositoryProfile['architecture'];

/** Layers with their globs, and the observed layer → layer dependency counts as a matrix. */
export function LayerMatrix({ architecture }: { architecture: Architecture }) {
  const { layers, matrix } = architecture;
  if (layers.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No layers were recognized (insufficient signal).
      </p>
    );
  }
  const counts = new Map(matrix.map((m) => [`${m.from}\u0000${m.to}`, m.count]));
  return (
    <div className="space-y-4">
      <ul className="grid gap-2 text-sm sm:grid-cols-2">
        {layers.map((l) => (
          <li key={l.name}>
            <span className="font-medium">{l.name}</span>{' '}
            <span className="text-muted-foreground">
              {l.member_count} members · {l.role_confidence.toFixed(2)}
            </span>
            <p className="font-mono text-xs text-muted-foreground">{l.globs.join(', ')}</p>
          </li>
        ))}
      </ul>
      <div className="overflow-x-auto">
        <table className="text-center text-xs" aria-label="Layer dependency matrix">
          <thead>
            <tr>
              <th className="p-1 text-left text-muted-foreground">from \ to</th>
              {layers.map((l) => (
                <th key={l.name} className="p-1 font-medium">
                  {l.name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {layers.map((from) => (
              <tr key={from.name}>
                <th className="p-1 text-left font-medium">{from.name}</th>
                {layers.map((to) => {
                  const n = counts.get(`${from.name}\u0000${to.name}`) ?? 0;
                  return (
                    <td
                      key={to.name}
                      className={`border p-1 tabular-nums ${n === 0 ? 'text-muted-foreground' : 'bg-primary/10'}`}
                    >
                      {n}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
