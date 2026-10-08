'use client';

import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import type { ReactNode } from 'react';
import { RunStateBadge } from '@/components/pulls/RunStateBadge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Select } from '@/components/ui/input';
import { ErrorState, Loading } from '@/components/ui/states';
import type { ReviewDetail } from '@/lib/api/pending';
import { formatDateTime, shortSha } from '@/lib/format';
import { reviewFindingsQuery, reviewHistoryQuery, reviewQuery } from '@/lib/queries';
import { isTerminalReview, runStateDisplay } from '@/lib/run-state';
import { ChangedFiles, EvidenceSummary } from './ChangedFiles';
import { ChangeSummary } from './ChangeSummary';
import { Completeness } from './Completeness';
import { FindingsList } from './FindingsList';
import { RiskSummary } from './RiskSummary';
import { StageTimeline } from './StageTimeline';
import { TraceLink } from './TraceLink';

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <Card id={id} aria-labelledby={`${id}-title`}>
      <CardHeader>
        <CardTitle id={`${id}-title`}>{title}</CardTitle>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

/** Other runs for the same pull request (superseded and earlier runs). */
function HistorySelect({ review }: { review: ReviewDetail }) {
  const router = useRouter();
  const history = useQuery(reviewHistoryQuery(review.pull_request.id));
  const runs = history.data ?? [];
  if (runs.length <= 1) return null;
  return (
    <label className="flex items-center gap-2 text-sm">
      <span className="text-muted-foreground">Review history</span>
      <Select
        aria-label="Review history"
        className="w-auto"
        value={review.id}
        onChange={(e) => router.push(`/reviews/${e.target.value}`)}
      >
        {runs.map((run) => (
          <option key={run.id} value={run.id}>
            {formatDateTime(run.created_at)} · {shortSha(run.head_sha)} ·{' '}
            {runStateDisplay(run.state, run.degraded).label}
          </option>
        ))}
      </Select>
    </label>
  );
}

/**
 * Review Detail. Read-only by design: there are no approve or publish controls on this page.
 */
export function ReviewDetailView({ reviewId }: { reviewId: string }) {
  const review = useQuery(reviewQuery(reviewId));
  const live = !!review.data && !isTerminalReview(review.data.state);
  const findings = useQuery(reviewFindingsQuery(reviewId, live));

  if (review.isPending) return <Loading label="Loading review" className="h-64" />;
  if (review.isError) {
    return <ErrorState error={review.error} onRetry={() => void review.refetch()} />;
  }
  const r = review.data;
  const pr = r.pull_request;
  const items = findings.data ?? [];

  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <p className="text-sm text-muted-foreground">
          <Link href={`/repositories/${pr.repository_id}`} className="hover:underline">
            {pr.repository_full_name}
          </Link>{' '}
          · {pr.base_ref} ← {pr.head_ref} · {shortSha(pr.head_sha)}
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="text-2xl font-semibold">
            #{pr.number} {pr.title}
          </h1>
          <RunStateBadge state={r.state} degraded={r.degraded} />
        </div>
        <HistorySelect review={r} />
      </div>

      <Section id="summary" title="Summary">
        <div className="grid gap-6 lg:grid-cols-2">
          <div className="space-y-3 text-sm">
            <dl className="grid grid-cols-[8rem_1fr] gap-y-1">
              <dt className="text-muted-foreground">Author</dt>
              <dd>{pr.author}</dd>
              <dt className="text-muted-foreground">Trigger</dt>
              <dd className="capitalize">{r.trigger}</dd>
              <dt className="text-muted-foreground">Started</dt>
              <dd>{formatDateTime(r.created_at)}</dd>
              <dt className="text-muted-foreground">Finished</dt>
              <dd>{formatDateTime(r.finished_at)}</dd>
              <dt className="text-muted-foreground">Trace</dt>
              <dd>
                <TraceLink traceId={r.trace_id} />
              </dd>
            </dl>
            {r.degraded_reasons.length > 0 && (
              <div className="rounded-md border border-amber-500/40 p-3">
                <p className="font-medium">Degraded</p>
                <ul className="list-disc pl-5 text-muted-foreground">
                  {r.degraded_reasons.map((reason) => (
                    <li key={reason}>{reason}</li>
                  ))}
                </ul>
              </div>
            )}
            <StageTimeline stages={r.stages} failure={r.failure} />
          </div>
          <Completeness completeness={r.completeness} reviewerRuns={r.reviewer_runs} />
        </div>
      </Section>

      <div className="grid gap-6 lg:grid-cols-2">
        <Section id="change" title="Change">
          <ChangeSummary change={r.change_summary} />
          {r.coverage && r.coverage.unreviewed_clusters.length > 0 && (
            <div className="mt-4 text-sm">
              <p className="font-medium">
                Unreviewed clusters ({r.coverage.unreviewed_clusters.length} of{' '}
                {r.coverage.reviewed_clusters + r.coverage.unreviewed_clusters.length})
              </p>
              <ul className="list-disc pl-5 text-muted-foreground">
                {r.coverage.unreviewed_clusters.map((c) => (
                  <li key={c.id}>
                    {c.reason}: {c.files.join(', ')}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </Section>
        <Section id="risk" title="Risk">
          <RiskSummary risk={r.risk} />
        </Section>
      </div>

      <Section id="findings" title="Findings">
        {findings.isPending && <Loading label="Loading findings" />}
        {findings.isError && (
          <ErrorState error={findings.error} onRetry={() => void findings.refetch()} />
        )}
        {findings.isSuccess && <FindingsList findings={items} />}
      </Section>

      <div className="grid gap-6 lg:grid-cols-2">
        <Section id="files" title="Files">
          <ChangedFiles change={r.change_summary} findings={items} />
        </Section>
        <Section id="evidence" title="Evidence">
          <EvidenceSummary findings={items} />
        </Section>
      </div>
    </div>
  );
}
