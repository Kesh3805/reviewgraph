import { Inject, Injectable } from '@nestjs/common';
import { incCounter } from '../../common/metrics';
import { hasMarker, parseFindingMarker, runIdOfMarker } from '../common/markers';
import {
  ProviderError,
  type CheckRunRequest,
  type ExistingReview,
  type InlineReviewComment,
  type PrRef,
  type PublishRequest,
  type PublishResult,
  type PublishedComment,
  type ResolveResult,
  type ReviewPublisher,
} from '../ports';
import { BITBUCKET_API, BitbucketApi, prPath, repoPath } from './bitbucket-auth.service';

/** Build status key (stable per repository) and the name Bitbucket shows. */
export const BITBUCKET_STATUS_KEY = 'reviewgraph';
export const BITBUCKET_STATUS_NAME = 'ReviewGraph';
const DESCRIPTION_MAX = 255;
/** Prefix of a comment that was marked instead of resolved (no resolve capability). */
export const RESOLVED_NOTE = '> Resolved by ReviewGraph (this server cannot resolve threads).';

interface BbComment {
  id: number;
  content: { raw: string };
  inline?: { path: string; from?: number | null; to?: number | null; outdated?: boolean } | null;
  parent?: { id: number } | null;
  user?: { uuid?: string };
  deleted?: boolean;
  resolution?: unknown;
}

type BbState = 'INPROGRESS' | 'SUCCESSFUL' | 'FAILED' | 'STOPPED';

/**
 * Build state for a check-run request. `FAILED` is deliberately unreachable: the review state
 * never blocks a merge (same rule as the GitHub check conclusions).
 */
export function buildStateOf(req: Pick<CheckRunRequest, 'status' | 'conclusion'>): BbState {
  if (req.status !== 'completed') return 'INPROGRESS';
  return req.conclusion === 'cancelled' ? 'STOPPED' : 'SUCCESSFUL';
}

/**
 * The Bitbucket Cloud `ReviewPublisher` (MP-002). Same publication discipline as the GitLab
 * adapter: inline comments first and the summary comment LAST (so the summary's existence
 * means "finished"), findings already on the pull request are skipped by their marker (a retry
 * never double-posts), and an unanchorable finding degrades to a general comment naming its
 * location. Bitbucket anchors by line number on a side (`to` = destination file, `from` = the
 * source side), not by diff position. A missing capability degrades visibly instead of failing
 * the run. No merge, approve or decline endpoint is referenced.
 */
@Injectable()
export class BitbucketReviewPublisher implements ReviewPublisher {
  private ownUuid?: string;

  constructor(@Inject(BITBUCKET_API) private readonly api: BitbucketApi | null) {}

  async publishReview(req: PublishRequest): Promise<PublishResult> {
    // The type already forbids anything else; this guards a cast at runtime (INV-011).
    if (req.event !== 'COMMENT') throw new ProviderError('invalid', 'only COMMENT reviews');
    const api = this.requireApi();
    const runId = runIdOfMarker(req.marker);
    const posted = new Map<string, string>(); // finding key -> comment id
    let summary: BbComment | undefined;
    for (const c of await this.comments(req.pr)) {
      if (!c.inline && hasMarker(c.content.raw, req.marker)) summary ??= c;
      const m = parseFindingMarker(c.content.raw);
      if (m && (runId === null || m.run === runId)) posted.set(m.finding, String(c.id));
    }

    const out: PublishedComment[] = [];
    for (const c of req.comments) {
      const key = parseFindingMarker(c.body)?.finding;
      const id = (key ? posted.get(key) : undefined) ?? (await this.postInline(req.pr, c));
      out.push({ findingId: c.findingId, providerCommentId: id });
    }

    let summaryId: string;
    if (summary) {
      summaryId = String(summary.id);
    } else {
      const raw = hasMarker(req.summary, req.marker)
        ? req.summary
        : `${req.summary}\n\n<!-- ${req.marker} -->`;
      const created = await api.post<BbComment>(`${prPath(req.pr)}/comments`, 'summary comment', {
        content: { raw },
      });
      summaryId = String(created.id);
    }
    return {
      providerReviewId: summaryId,
      url: `https://bitbucket.org/${req.pr.owner}/${req.pr.name}/pull-requests/${req.pr.number}#comment-${summaryId}`,
      comments: out,
    };
  }

  async findExistingReview(ref: PrRef, marker: string): Promise<ExistingReview | null> {
    const runId = runIdOfMarker(marker);
    let summary: BbComment | undefined;
    const comments: PublishedComment[] = [];
    for (const c of await this.comments(ref)) {
      if (!c.inline && hasMarker(c.content.raw, marker)) summary ??= c;
      const m = parseFindingMarker(c.content.raw);
      if (m && (runId === null || m.run === runId)) {
        comments.push({ findingId: m.finding, providerCommentId: String(c.id) });
      }
    }
    return summary ? { providerReviewId: String(summary.id), comments } : null;
  }

  async upsertCheckRun(req: CheckRunRequest): Promise<{ checkRunId: string }> {
    const api = this.requireApi();
    const body = {
      key: BITBUCKET_STATUS_KEY,
      name: req.name || BITBUCKET_STATUS_NAME,
      state: buildStateOf(req),
      description: `${req.title}: ${req.summary}`.slice(0, DESCRIPTION_MAX),
      url: `https://bitbucket.org/${req.repo.owner}/${req.repo.name}/commits/${req.headSha}`,
    };
    const base = `${repoPath(req.repo)}/commit/${encodeURIComponent(req.headSha)}/statuses/build`;
    // A status is keyed by (commit, key): update it when it exists, create it otherwise.
    const updated = await api.put(
      `${base}/${BITBUCKET_STATUS_KEY}`,
      'build status update',
      body,
      [404],
    );
    if (updated.status === 404) await api.post(base, 'build status creation', body);
    return { checkRunId: `${req.headSha}:${BITBUCKET_STATUS_KEY}` };
  }

  /**
   * Resolves the comments that are ours (the token's own account) and carry one of our finding
   * markers. Human comments are never touched. Without the resolve capability (Data Center,
   * older servers) the comment text is marked instead, with a visible reason.
   */
  async resolveThreads(ref: PrRef, providerCommentIds: string[]): Promise<ResolveResult> {
    const result: ResolveResult = { resolved: [], skipped: [] };
    if (providerCommentIds.length === 0) return result;
    const api = this.requireApi();
    const me = await this.me();
    const byId = new Map((await this.comments(ref)).map((c) => [String(c.id), c]));
    for (const id of providerCommentIds) {
      const c = byId.get(id);
      if (!c || c.user?.uuid !== me || !parseFindingMarker(c.content.raw)) {
        result.skipped.push(id);
        continue;
      }
      if (c.resolution) {
        result.resolved.push(id);
        continue;
      }
      try {
        await api.post(`${prPath(ref)}/comments/${id}/resolve`, 'comment resolve', {});
      } catch (err) {
        if (!(err instanceof ProviderError) || !['not_found', 'invalid'].includes(err.kind)) {
          throw err;
        }
        incCounter('provider_capability_degraded_total', {
          provider: 'bitbucket',
          capability: 'resolve_thread',
        });
        if (!c.content.raw.startsWith(RESOLVED_NOTE)) {
          await api.put(`${prPath(ref)}/comments/${id}`, 'comment edit', {
            content: { raw: `${RESOLVED_NOTE}\n\n${c.content.raw}` },
          });
        }
      }
      result.resolved.push(id);
    }
    return result;
  }

  /** One inline comment at the finding's line on the destination (RIGHT) or source (LEFT) side. */
  private async postInline(pr: PrRef, c: InlineReviewComment): Promise<string> {
    const api = this.requireApi();
    const path = `${prPath(pr)}/comments`;
    const inline =
      c.side === 'LEFT' ? { path: c.path, from: c.line } : { path: c.path, to: c.line };
    try {
      return String(
        (await api.post<BbComment>(path, 'inline comment', { content: { raw: c.body }, inline }))
          .id,
      );
    } catch (err) {
      if (!(err instanceof ProviderError) || err.kind !== 'invalid') throw err;
    }
    // Not anchorable (line outside the diff): keep the finding visible, with its location.
    incCounter('provider_capability_degraded_total', {
      provider: 'bitbucket',
      capability: 'inline_anchor',
    });
    const raw = `**${c.path}:${c.line}** (could not be anchored to the diff)\n\n${c.body}`;
    const general = await api.post<BbComment>(path, 'general comment', { content: { raw } });
    return String(general.id);
  }

  private async comments(ref: PrRef): Promise<BbComment[]> {
    const out: BbComment[] = [];
    for await (const c of this.requireApi().pages<BbComment>(
      `${prPath(ref)}/comments`,
      'comment listing',
    )) {
      if (!c.deleted) out.push(c);
    }
    return out;
  }

  private async me(): Promise<string | undefined> {
    if (this.ownUuid === undefined) {
      this.ownUuid = (
        await this.requireApi().get<{ uuid: string }>('/user', 'current user lookup')
      ).uuid;
    }
    return this.ownUuid;
  }

  private requireApi(): BitbucketApi {
    if (!this.api) throw new ProviderError('forbidden', 'Bitbucket integration is disabled');
    return this.api;
  }
}
