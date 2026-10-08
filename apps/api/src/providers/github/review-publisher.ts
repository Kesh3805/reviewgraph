import { Inject, Injectable } from '@nestjs/common';
import { APP_CONFIG, type AppConfig } from '../../config/config.module';
import {
  ProviderError,
  type CheckRunRequest,
  type ExistingReview,
  type PrRef,
  type PublishRequest,
  type PublishResult,
  type PublishedComment,
  type ResolveResult,
  type ReviewPublisher,
} from '../ports';
import type { GithubAppAuth } from './app-auth.service';
import { githubCall } from './api-call';
import { GITHUB_APP_AUTH } from './github.tokens';
import { isAppAuthor, listReviewThreads, resolveReviewThread } from './graphql';
import type { GithubOctokit } from './octokit.factory';

/** The review POST holds the publish lock (SUP-003), so it is bounded. */
export const REVIEW_POST_TIMEOUT_MS = 15_000;
const PER_PAGE = 100;
const MAX_PAGES = 30;

const FINDING_MARKER = /<!-- reviewgraph:finding=([A-Za-z0-9._:-]+) run=([A-Za-z0-9._:-]+) -->/;

/** The finding key (fingerprint) and run id of an inline comment's hidden marker. */
export function parseFindingMarker(body: string): { finding: string; run: string } | null {
  const m = FINDING_MARKER.exec(body);
  return m?.[1] && m[2] ? { finding: m[1], run: m[2] } : null;
}

/** True when `body` carries `marker` as a whole token (e.g. `reviewgraph:run=<id>`). */
export function hasMarker(body: string | null | undefined, marker: string): boolean {
  if (!body) return false;
  let from = 0;
  for (;;) {
    const at = body.indexOf(marker, from);
    if (at < 0) return false;
    const next = body.charAt(at + marker.length);
    if (next === '' || next === ' ' || next === '-' || next === '\n') return true;
    from = at + 1;
  }
}

interface GhReview {
  id: number;
  body?: string | null;
  html_url?: string;
  commit_id?: string;
}

interface GhReviewComment {
  id: number;
  body: string;
}

/**
 * The GitHub `ReviewPublisher` (GH-009, GH-011): one atomic `COMMENT` review per call, marker
 * based adoption of an already-posted review, check runs that never block a merge, and
 * resolution of the App's own stale review threads.
 */
@Injectable()
export class GithubReviewPublisher implements ReviewPublisher {
  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    @Inject(GITHUB_APP_AUTH) private readonly auth: GithubAppAuth | null,
  ) {}

  async publishReview(req: PublishRequest): Promise<PublishResult> {
    // The type already forbids anything else; this guards a cast at runtime (INV-011).
    if (req.event !== 'COMMENT') throw new ProviderError('invalid', 'only COMMENT reviews');
    const octokit = await this.octokit(req.pr);
    const body = hasMarker(req.summary, req.marker)
      ? req.summary
      : `${req.summary}\n\n<!-- ${req.marker} -->`;
    const { data } = await githubCall<GhReview>(
      'POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews',
      'review publication',
      async () =>
        (await octokit.request('POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews', {
          owner: req.pr.owner,
          repo: req.pr.name,
          pull_number: req.pr.number,
          commit_id: req.headSha,
          body,
          event: 'COMMENT',
          comments: req.comments.map((c) => ({
            path: c.path,
            line: c.line,
            side: c.side,
            ...(c.startLine !== undefined && c.startLine < c.line
              ? { start_line: c.startLine, start_side: c.startSide ?? c.side }
              : {}),
            body: c.body,
          })),
          // Never retried in-process: a POST whose response was lost may have created the
          // review, and only the marker lookup of the next attempt can tell (no duplicates).
          request: { signal: AbortSignal.timeout(REVIEW_POST_TIMEOUT_MS), retries: 0 },
        })) as { status: number; data: GhReview },
      { span: 'github_create_review', attributes: { 'github.pr': req.pr.number } },
    );
    const reviewId = String(data.id);
    // Map by the marker of each comment we sent, so ids line up with our findings.
    const byKey = new Map<string, string>();
    for (const c of req.comments) {
      const marker = parseFindingMarker(c.body);
      if (marker) byKey.set(marker.finding, c.findingId);
    }
    const comments = (await this.reviewComments(octokit, req.pr, reviewId)).map((c) => ({
      ...c,
      findingId: byKey.get(c.findingId) ?? c.findingId,
    }));
    return {
      providerReviewId: reviewId,
      ...(data.html_url ? { url: data.html_url } : {}),
      comments,
    };
  }

  async findExistingReview(ref: PrRef, marker: string): Promise<ExistingReview | null> {
    const octokit = await this.octokit(ref);
    for (let page = 1; page <= MAX_PAGES; page++) {
      const { data } = await githubCall<GhReview[]>(
        'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews',
        'review listing',
        async () =>
          (await octokit.request('GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews', {
            owner: ref.owner,
            repo: ref.name,
            pull_number: ref.number,
            per_page: PER_PAGE,
            page,
          })) as { status: number; data: GhReview[] },
      );
      const found = data.find((r) => hasMarker(r.body, marker));
      if (found) {
        const id = String(found.id);
        return { providerReviewId: id, comments: await this.reviewComments(octokit, ref, id) };
      }
      if (data.length < PER_PAGE) return null;
    }
    return null;
  }

  async upsertCheckRun(req: CheckRunRequest): Promise<{ checkRunId: string }> {
    const octokit = await this.octokit(req.repo);
    const fields = {
      name: req.name,
      status: req.status,
      ...(req.status === 'completed' && req.conclusion ? { conclusion: req.conclusion } : {}),
      ...(req.externalId ? { external_id: req.externalId } : {}),
      output: { title: req.title, summary: req.summary },
    };
    if (req.checkRunId) {
      const { data } = await githubCall<{ id: number }>(
        'PATCH /repos/{owner}/{repo}/check-runs/{check_run_id}',
        'check run update',
        async () =>
          (await octokit.request('PATCH /repos/{owner}/{repo}/check-runs/{check_run_id}', {
            owner: req.repo.owner,
            repo: req.repo.name,
            check_run_id: Number(req.checkRunId),
            ...fields,
          })) as { status: number; data: { id: number } },
        { span: 'github_check_run' },
      );
      return { checkRunId: String(data.id) };
    }
    const { data } = await githubCall<{ id: number }>(
      'POST /repos/{owner}/{repo}/check-runs',
      'check run creation',
      async () =>
        (await octokit.request('POST /repos/{owner}/{repo}/check-runs', {
          owner: req.repo.owner,
          repo: req.repo.name,
          head_sha: req.headSha,
          ...fields,
        })) as { status: number; data: { id: number } },
      { span: 'github_check_run' },
    );
    return { checkRunId: String(data.id) };
  }

  /**
   * Resolves the threads whose first comment is one of `providerCommentIds` AND was written by
   * this App. Human threads are never touched; already-resolved threads count as resolved.
   */
  async resolveThreads(ref: PrRef, providerCommentIds: string[]): Promise<ResolveResult> {
    const result: ResolveResult = { resolved: [], skipped: [] };
    if (providerCommentIds.length === 0) return result;
    const octokit = await this.octokit(ref);
    const threads = await listReviewThreads(octokit, ref);
    const byComment = new Map(
      threads.filter((t) => t.firstCommentId).map((t) => [t.firstCommentId!, t]),
    );
    for (const commentId of providerCommentIds) {
      const thread = byComment.get(commentId);
      if (!thread || !isAppAuthor(thread.firstCommentAuthor, this.config.GITHUB_APP_SLUG)) {
        result.skipped.push(commentId);
        continue;
      }
      if (!thread.isResolved) await resolveReviewThread(octokit, thread.id);
      result.resolved.push(commentId);
    }
    return result;
  }

  /** Comments of one review; `findingId` is the marker's finding key (the fingerprint). */
  private async reviewComments(
    octokit: GithubOctokit,
    ref: PrRef,
    reviewId: string,
  ): Promise<PublishedComment[]> {
    const out: PublishedComment[] = [];
    for (let page = 1; page <= MAX_PAGES; page++) {
      const { data } = await githubCall<GhReviewComment[]>(
        'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews/{review_id}/comments',
        'review comment listing',
        async () =>
          (await octokit.request(
            'GET /repos/{owner}/{repo}/pulls/{pull_number}/reviews/{review_id}/comments',
            {
              owner: ref.owner,
              repo: ref.name,
              pull_number: ref.number,
              review_id: Number(reviewId),
              per_page: PER_PAGE,
              page,
            },
          )) as { status: number; data: GhReviewComment[] },
      );
      for (const c of data) {
        const marker = parseFindingMarker(c.body);
        if (marker) out.push({ findingId: marker.finding, providerCommentId: String(c.id) });
      }
      if (data.length < PER_PAGE) break;
    }
    return out;
  }

  private octokit(ref: PrRef | { installationId: string }): Promise<GithubOctokit> {
    if (!this.auth) throw new ProviderError('forbidden', 'GitHub integration is disabled');
    return this.auth.getOctokit(ref.installationId);
  }
}
