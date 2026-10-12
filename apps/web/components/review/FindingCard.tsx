import Link from 'next/link';
import type { ReactNode } from 'react';
import type { FindingSummary } from '@/lib/api/pending';
import { formatPercent } from '@/lib/format';
import { SEVERITY_TOKENS } from '@/lib/severity';

export function SeverityLabel({ severity }: { severity: FindingSummary['severity'] }) {
  const token = SEVERITY_TOKENS[severity];
  return (
    <span className={`inline-flex items-center gap-1 text-xs font-medium ${token.text}`}>
      <span aria-hidden className={`size-2 rounded-full ${token.dot}`} />
      {token.label}
    </span>
  );
}

/** One finding: severity, title (links to Finding Detail), anchor, reviewer and confidence. */
export function FindingCard({
  finding,
  children,
}: {
  finding: FindingSummary;
  children?: ReactNode;
}) {
  return (
    <li className="space-y-1 px-3 py-2" data-testid={`finding-${finding.id}`}>
      <div className="flex flex-wrap items-center gap-2">
        <SeverityLabel severity={finding.severity} />
        <Link href={`/findings/${finding.id}`} className="font-medium hover:underline">
          {finding.title}
        </Link>
      </div>
      <p className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
        <span className="font-mono">
          {finding.anchor.path}:{finding.anchor.start_line}
        </span>
        <span>
          {finding.reviewer}:{finding.reviewer_version}
        </span>
        <span>confidence {formatPercent(finding.confidence)}</span>
        {finding.suppression_reason && <span>reason: {finding.suppression_reason}</span>}
      </p>
      {children}
    </li>
  );
}
