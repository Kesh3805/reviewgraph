import Link from 'next/link';
import type { ChangeSummary, FindingSummary } from '@/lib/api/pending';
import { findingsByPath } from '@/lib/findings';
import { SeverityLabel } from './FindingCard';

/** Changed files with the findings anchored in each. */
export function ChangedFiles({
  change,
  findings,
}: {
  change: ChangeSummary | null;
  findings: FindingSummary[];
}) {
  if (!change) return <p className="text-sm text-muted-foreground">No file list available.</p>;
  // Published findings only; suppressed ones live in the Findings section.
  const byPath = findingsByPath(findings.filter((f) => f.state === 'PUBLISHED'));
  return (
    <ul aria-label="Changed files" className="divide-y rounded-md border text-sm">
      {change.files.map((file) => {
        const here = byPath.get(file.path) ?? [];
        return (
          <li key={file.path} className="px-3 py-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="font-mono text-xs break-all">
                {file.old_path && file.status === 'renamed' ? `${file.old_path} → ` : ''}
                {file.path}
              </span>
              <span className="flex items-center gap-3 text-xs">
                <span className="text-muted-foreground capitalize">{file.status}</span>
                <span className="text-emerald-700 tabular-nums dark:text-emerald-400">
                  +{file.additions}
                </span>
                <span className="text-red-700 tabular-nums dark:text-red-400">
                  -{file.deletions}
                </span>
                <span className="tabular-nums">{here.length} findings</span>
              </span>
            </div>
            {here.length > 0 && (
              <ul className="mt-1 space-y-0.5 pl-3">
                {here.map((f) => (
                  <li key={f.id} className="flex items-center gap-2 text-xs">
                    <SeverityLabel severity={f.severity} />
                    <Link href={`/findings/${f.id}`} className="hover:underline">
                      {f.title}
                    </Link>
                    <span className="text-muted-foreground">line {f.anchor.start_line}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/** Per-finding evidence summary, linking to Finding Detail. */
export function EvidenceSummary({ findings }: { findings: FindingSummary[] }) {
  const withEvidence = findings.filter((f) => f.state === 'PUBLISHED');
  if (withEvidence.length === 0) {
    return <p className="text-sm text-muted-foreground">No published findings.</p>;
  }
  return (
    <ul aria-label="Evidence" className="space-y-2 text-sm">
      {withEvidence.map((f) => (
        <li key={f.id}>
          <Link href={`/findings/${f.id}`} className="font-medium hover:underline">
            {f.title}
          </Link>
          <p className="text-muted-foreground">
            {f.evidence_count} evidence items
            {f.evidence_summary ? ` · ${f.evidence_summary}` : ''}
          </p>
        </li>
      ))}
    </ul>
  );
}
