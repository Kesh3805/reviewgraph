'use client';

import { Bar, BarChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';

export function histogramBuckets(counts: number[]): { bucket: string; count: number }[] {
  return Array.from({ length: 10 }, (_, i) => ({
    bucket: `${(i / 10).toFixed(1)}–${((i + 1) / 10).toFixed(1)}`,
    count: counts[i] ?? 0,
  }));
}

/** Edge confidence in 10 buckets, with the same data as a table for assistive technology. */
export function ConfidenceHistogram({ counts }: { counts: number[] }) {
  const data = histogramBuckets(counts);
  return (
    <div>
      <div className="h-40" aria-hidden>
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
            <XAxis dataKey="bucket" tick={{ fontSize: 10 }} interval={1} />
            <YAxis tick={{ fontSize: 10 }} width={40} allowDecimals={false} />
            <Tooltip />
            <Bar dataKey="count" fill="currentColor" className="text-primary" />
          </BarChart>
        </ResponsiveContainer>
      </div>
      <table className="sr-only" aria-label="Edge confidence histogram">
        <thead>
          <tr>
            <th>Confidence</th>
            <th>Edges</th>
          </tr>
        </thead>
        <tbody>
          {data.map((d) => (
            <tr key={d.bucket}>
              <td>{d.bucket}</td>
              <td>{d.count}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
