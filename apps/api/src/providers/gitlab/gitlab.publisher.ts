import { Inject, Injectable } from '@nestjs/common';
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
import { GITLAB_API, GitlabApi, mrPath, projectPath } from './gitlab-auth.service';
import type { GlMergeRequest } from './gitlab.mapping';

/** Commit status name shown on the merge request (and in the pipeline list). */
export const GITLAB_STATUS_NAME = 'ReviewGraph';
/** GitLab truncates status descriptions at 255 characters. */
const DESCRIPTION_MAX = 255;

interface GlNote {
  id: number;
  body: string;
  author?: { id?: number; username?: string };
  resolved?: boolean;
  position?: unknown;
}

interface GlDiscussion {
  id: string;
  individual_note?: boolean;
  notes: GlNote[];
}

type GlState = 'pending' | 'running' | 'success' | 'failed' | 'canceled' | 'skipped';

/**
 * Commit status for a check-run request. `failed` is deliberately unreachable: the review
 * state never blocks a merge (same rule as the GitHub check conclusions).
 */
export function statusStateOf(req: Pick<CheckRunRequest, 'status' | 'conclusion'>): GlState {
  if (req.status === 'queued') return 'pending';
  if (req.status === 'in_progress') return 'running';
  if (req.conclusion === 'cancelled') return 'canceled';
  if (req.conclusion === 'skipped') return 'skipped';
  return 'success';
}

/**
 * The GitLab `ReviewPublisher` (MP-001). One summary note and one discussion per inline
 * finding, posted sequentially and resumably:
 *  - inline discussions go first and the summary note LAST, so "the summary exists" means the
 *    publication finished and `findExistingReview` can safely adopt it;
 *  - before posting, existing discussions are read and every finding whose marker is already
 *    on the merge request is skipped, so a retry never double-posts;
 *  - a finding GitLab cannot anchor (the line is not in the diff) degrades to a general
 *    discussion that names the location, instead of failing the run.
 * `providerCommentId` is the discussion id (what the `resolved` flag lives on). The adapter
 * calls no merge or approve endpoint: the only writes are notes, discussions, resolving own
 * discussions and commit statuses.
 */
@Injectable()
export class GitlabReviewPublisher implements ReviewPublisher {
  private ownUserId?: number;

  constructor(@Inject(GITLAB_API) private readonly api: GitlabApi | null) {}

  async publishReview(req: PublishRequest): Promise<PublishResult> {
    // The type already forbids anything else; this guards a cast at runtime (INV-011).
    if (req.event !== 'COMMENT') throw new ProviderError('invalid', 'only COMMENT reviews');
    const api = this.requireApi();
    const mr = await api.get<GlMergeRequest>(mrPath(req.pr), 'merge request lookup');
    const refs = mr.diff_refs;
    if (!refs || refs.head_sha !== req.headSha) {
      throw new ProviderError('invalid', 'merge request head moved since the review was made');
    }

    const runId = runIdOfMarker(req.marker);
    const existing = await this.discussions(req.pr);
    const posted = new Map<string, string>(); // finding key -> discussion id
    let summary: GlNote | undefined;
    for (const d of existing) {
      const first = d.notes[0];
      if (!first) continue;
      if (hasMarker(first.body, req.marker)) summary ??= first;
      const m = parseFindingMarker(first.body);
      if (m && (runId === null || m.run === runId)) posted.set(m.finding, d.id);
    }

    const out: PublishedComment[] = [];
    for (const c of req.comments) {
      const key = parseFindingMarker(c.body)?.finding;
      const known = key ? posted.get(key) : undefined;
      const id =
        known ??
        (await this.postDiscussion(req.pr, c, {
          base_sha: refs.base_sha,
          start_sha: refs.start_sha,
          head_sha: refs.head_sha,
        }));
      out.push({ findingId: c.findingId, providerCommentId: id });
    }

    let summaryId: string;
    if (summary) {
      summaryId = String(summary.id);
    } else {
      const body = hasMarker(req.summary, req.marker)
        ? req.summary
        : `${req.summary}\n\n<!-- ${req.marker} -->`;
      const note = await api.post<GlNote>(`${mrPath(req.pr)}/notes`, 'summary note creation', {
        body,
      });
      summaryId = String(note.id);
    }
    return {
      providerReviewId: summaryId,
      url: `${api.baseUrl}/${req.pr.owner}/${req.pr.name}/-/merge_requests/${req.pr.number}#note_${summaryId}`,
      comments: out,
    };
  }

  async findExistingReview(ref: PrRef, marker: string): Promise<ExistingReview | null> {
    const runId = runIdOfMarker(marker);
    let summary: GlNote | undefined;
    const comments: PublishedComment[] = [];
    for (const d of await this.discussions(ref)) {
      const first = d.notes[0];
      if (!first) continue;
      if (hasMarker(first.body, marker)) summary ??= first;
      const m = parseFindingMarker(first.body);
      if (m && (runId === null || m.run === runId)) {
        comments.push({ findingId: m.finding, providerCommentId: d.id });
      }
    }
    return summary ? { providerReviewId: String(summary.id), comments } : null;
  }

  async upsertCheckRun(req: CheckRunRequest): Promise<{ checkRunId: string }> {
    const api = this.requireApi();
    // A status is keyed by (sha, name): posting again moves the same status along.
    await api.post(
      `${projectPath(req.repo)}/statuses/${encodeURIComponent(req.headSha)}`,
      'commit status update',
      {
        name: req.name || GITLAB_STATUS_NAME,
        state: statusStateOf(req),
        description: `${req.title}: ${req.summary}`.slice(0, DESCRIPTION_MAX),
      },
    );
    return { checkRunId: `${req.headSha}:${req.name || GITLAB_STATUS_NAME}` };
  }

  /**
   * Resolves the discussions whose first note is ours (the token's own user) and carries one
   * of our finding markers. Human discussions are never touched; an already-resolved one
   * counts as resolved.
   */
  async resolveThreads(ref: PrRef, providerCommentIds: string[]): Promise<ResolveResult> {
    const result: ResolveResult = { resolved: [], skipped: [] };
    if (providerCommentIds.length === 0) return result;
    const api = this.requireApi();
    const me = await this.me();
    const byId = new Map((await this.discussions(ref)).map((d) => [d.id, d]));
    for (const id of providerCommentIds) {
      const first = byId.get(id)?.notes[0];
      if (!first || first.author?.id !== me || !parseFindingMarker(first.body)) {
        result.skipped.push(id);
        continue;
      }
      if (first.resolved !== true) {
        await api.put(
          `${mrPath(ref)}/discussions/${encodeURIComponent(id)}`,
          'discussion resolve',
          {
            resolved: 'true',
          },
        );
      }
      result.resolved.push(id);
    }
    return result;
  }

  /** One discussion at the finding's line, anchored with the merge request's `diff_refs`. */
  private async postDiscussion(
    pr: PrRef,
    c: InlineReviewComment,
    refs: { base_sha: string; start_sha: string; head_sha: string },
  ): Promise<string> {
    const api = this.requireApi();
    const path = `${mrPath(pr)}/discussions`;
    const position = {
      position_type: 'text',
      ...refs,
      old_path: c.path,
      new_path: c.path,
      ...(c.side === 'LEFT' ? { old_line: c.line } : { new_line: c.line }),
    };
    try {
      const d = await api.post<GlDiscussion>(path, 'discussion creation', {
        body: c.body,
        position,
      });
      return d.id;
    } catch (err) {
      if (!(err instanceof ProviderError) || err.kind !== 'invalid') throw err;
    }
    // Not anchorable (line outside the diff): keep the finding visible, with its location.
    const where = `${c.path}:${c.line}`;
    const d = await api.post<GlDiscussion>(path, 'general discussion creation', {
      body: `**${where}** (could not be anchored to the diff)\n\n${c.body}`,
    });
    return d.id;
  }

  private async discussions(ref: PrRef): Promise<GlDiscussion[]> {
    const out: GlDiscussion[] = [];
    for await (const d of this.requireApi().pages<GlDiscussion>(
      `${mrPath(ref)}/discussions`,
      'discussion listing',
    )) {
      out.push(d);
    }
    return out;
  }

  private async me(): Promise<number | undefined> {
    if (this.ownUserId === undefined) {
      const user = await this.requireApi().get<{ id: number }>('/user', 'current user lookup');
      this.ownUserId = user.id;
    }
    return this.ownUserId;
  }

  private requireApi(): GitlabApi {
    if (!this.api) throw new ProviderError('forbidden', 'GitLab integration is disabled');
    return this.api;
  }
}
