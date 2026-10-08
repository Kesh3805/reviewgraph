'use client';

import { useQuery } from '@tanstack/react-query';
import { ExternalLink } from 'lucide-react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import type { ReactNode } from 'react';
import { FeedbackMenu } from '@/components/feedback/FeedbackMenu';
import { EffectivePolicy } from '@/components/profile/EffectivePolicy';
import { SeverityLabel } from '@/components/review/FindingCard';
import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { ErrorState, Loading } from '@/components/ui/states';
import type { FindingDetail, Publication } from '@/lib/api/pending';
import { formatDateTime, formatPercent } from '@/lib/format';
import { findingQuery, findingTraceQuery, sourceExcerptQuery } from '@/lib/queries';
import { BaseHeadCompare } from './BaseHeadCompare';
import { ConfidenceBreakdown } from './ConfidenceBreakdown';
import { ImpactPathFlow } from './ImpactPathFlow';
import { SourceExcerpt } from './SourceExcerpt';
import { VerificationStages } from './VerificationStages';

/** Lines of context shown around the anchor. */
const CONTEXT_LINES = 5;

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card aria-label={title}>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

function AnchorExcerpt({ finding }: { finding: FindingDetail }) {
  const { anchor } = finding;
  const excerpt = useQuery(
    sourceExcerptQuery(finding.repository_id, {
      path: anchor.path,
      start: Math.max(1, anchor.start_line - CONTEXT_LINES),
      end: anchor.end_line + CONTEXT_LINES,
      snapshot: finding.snapshots.head,
    }),
  );
  if (excerpt.isPending) return <Loading label="Loading source excerpt" />;
  if (excerpt.isError) {
    return <ErrorState error={excerpt.error} onRetry={() => void excerpt.refetch()} />;
  }
  return (
    <SourceExcerpt excerpt={excerpt.data} highlightLines={[anchor.start_line, anchor.end_line]} />
  );
}

function PublicationInfo({ publication }: { publication: Publication | null }) {
  if (!publication || publication.state === 'not_published') {
    return <p className="text-sm text-muted-foreground">Not published to the provider.</p>;
  }
  return (
    <p className="flex flex-wrap items-center gap-2 text-sm">
      <Badge variant={publication.state === 'resolved' ? 'muted' : 'success'}>
        {publication.state}
      </Badge>
      {formatDateTime(publication.published_at)}
      {publication.provider_comment_url && (
        <a
          href={publication.provider_comment_url}
          target="_blank"
          rel="noreferrer noopener"
          className="inline-flex items-center gap-1 hover:underline"
        >
          Provider comment <ExternalLink className="size-3" aria-hidden />
        </a>
      )}
    </p>
  );
}

/**
 * Finding Detail: why a comment exists, reconstructed from the trace. It never renders prompts
 * or raw model output (the API does not return them; only typed fields are rendered here).
 */
export function FindingDetailView({ findingId }: { findingId: string }) {
  const router = useRouter();
  const finding = useQuery(findingQuery(findingId));
  const trace = useQuery(findingTraceQuery(findingId));

  if (finding.isPending) return <Loading label="Loading finding" className="h-64" />;
  if (finding.isError) {
    return <ErrorState error={finding.error} onRetry={() => void finding.refetch()} />;
  }
  const f = finding.data;
  const t = trace.data;
  const openSymbol = (key: string) =>
    router.push(
      `/repositories/${f.repository_id}/graph?symbol=${encodeURIComponent(key)}&snapshot=${encodeURIComponent(f.snapshots.head)}`,
    );

  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <p className="text-sm text-muted-foreground">
          <Link href={`/reviews/${f.review_id}`} className="hover:underline">
            {f.pull_request.repository_full_name}#{f.pull_request.number} {f.pull_request.title}
          </Link>
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <SeverityLabel severity={f.severity} />
          <h1 className="text-2xl font-semibold">{f.title}</h1>
        </div>
        <p className="flex flex-wrap gap-x-4 text-sm text-muted-foreground">
          <span>
            {f.reviewer}@{f.reviewer_version}
          </span>
          <span>{f.category}</span>
          <span>{f.state}</span>
          <span>confidence {formatPercent(f.confidence)}</span>
        </p>
        <p className="max-w-3xl text-sm whitespace-pre-line">{f.explanation}</p>
      </div>

      <FeedbackMenu findingId={f.id} />

      <div className="grid gap-6 lg:grid-cols-2">
        <Section title="Confidence">
          <ConfidenceBreakdown
            confidence={f.confidence}
            components={f.confidence_components}
            verificationVersion={f.verification_version}
          />
        </Section>
        <Section title="Anchor">
          <p className="mb-2 font-mono text-xs">
            {f.anchor.path}:{f.anchor.start_line}-{f.anchor.end_line}
          </p>
          <AnchorExcerpt finding={f} />
        </Section>
      </div>

      {trace.isPending && <Loading label="Loading trace" />}
      {trace.isError && (
        <ErrorState
          title="Could not load the trace."
          error={trace.error}
          onRetry={() => void trace.refetch()}
        />
      )}
      {t && (
        <>
          {t.incomplete && (
            <p
              role="status"
              className="rounded-md border border-amber-500/40 p-3 text-sm text-muted-foreground"
            >
              This trace is incomplete: some stages were not recorded for this run. The available
              stages are shown.
            </p>
          )}

          <Section title="Impact path">
            {t.impact_path && t.impact_path.nodes.length > 0 ? (
              <ImpactPathFlow path={t.impact_path} onSymbolClick={openSymbol} />
            ) : (
              <p className="text-sm text-muted-foreground">No impact path recorded.</p>
            )}
          </Section>

          <Section title="Base and head">
            {t.base_head ? (
              <BaseHeadCompare
                repoId={f.repository_id}
                base={t.base_head.base}
                head={t.base_head.head}
              />
            ) : (
              <p className="text-sm text-muted-foreground">No base/head comparison recorded.</p>
            )}
          </Section>

          <Section title="Verification">
            <VerificationStages stages={t.verification} />
          </Section>

          <div className="grid gap-6 lg:grid-cols-2">
            <Section title="Deduplication">
              {t.dedup_merges.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No findings were merged into this one.
                </p>
              ) : (
                <ul className="space-y-1 text-sm">
                  {t.dedup_merges.map((m) => (
                    <li key={m.finding_id}>
                      <Link href={`/findings/${m.finding_id}`} className="hover:underline">
                        {m.title}
                      </Link>{' '}
                      <span className="text-muted-foreground">
                        {m.reviewer} · similarity {m.similarity.toFixed(2)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
            <Section title="Effective policy">
              <EffectivePolicy policies={t.effective_policy} />
            </Section>
          </div>
        </>
      )}

      <Section title="Publication">
        <PublicationInfo publication={f.publication ?? t?.publication ?? null} />
      </Section>
    </div>
  );
}
