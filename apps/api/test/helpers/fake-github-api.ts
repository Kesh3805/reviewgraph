import { createHash } from 'node:crypto';
import type { ServerResponse } from 'node:http';
import { FakeGithub, type FakeRequest } from './fake-github';

/**
 * Stateful fake of the GitHub REST/GraphQL surface used by sync, publication, stale resolution
 * and the reconciler (DEV-005 stand-in). It keeps repositories, pull requests, reviews, review
 * comments, review threads and check runs in memory, and supports fault injection:
 *  - `faults.reviewPost = 'drop_after_write'` stores the review, then drops the connection;
 *  - `faults.reviewPost = 422` answers "line must be part of the diff" when inline comments exist;
 *  - `faults.reviewPost = <status>` answers that status;
 *  - `faults.graphql = <status>` fails GraphQL calls;
 *  - `reviewPostDelayMs` delays the review POST (publish while a supersession waits).
 */
export interface FakePull {
  number: number;
  title: string;
  state: 'open' | 'closed';
  draft: boolean;
  merged: boolean;
  user: { login: string; type: string };
  head: { sha: string; ref: string };
  base: { sha: string; ref: string };
  updated_at: string;
  files: {
    filename: string;
    status: string;
    additions: number;
    deletions: number;
    patch?: string;
  }[];
}

export interface FakeReview {
  id: number;
  pull: number;
  repo: string;
  body: string;
  event: string;
  commit_id: string;
  comments: { id: number; path: string; line: number; side: string; body: string }[];
}

export interface FakeThread {
  id: string;
  repo: string;
  pull: number;
  isResolved: boolean;
  firstCommentId: number;
  author: string;
}

export class FakeGithubApi extends FakeGithub {
  readonly repos = new Map<string, { id: number; default_branch: string; private: boolean }>();
  readonly pulls = new Map<string, FakePull>();
  readonly reviews: FakeReview[] = [];
  readonly threads: FakeThread[] = [];
  readonly checkRuns: { id: number; repo: string; body: Record<string, unknown> }[] = [];
  readonly resolvedThreadIds: string[] = [];
  /** `compare` results: `${repo}:${base}...${head}` -> changed paths. */
  readonly compares = new Map<string, string[]>();
  faults: { reviewPost?: 'drop_after_write' | number; graphql?: number } = {};
  reviewPostDelayMs = 0;
  /** App bot login in GraphQL form (no `[bot]` suffix). */
  botLogin = 'reviewgraph';
  private nextId = 1000;

  constructor(publicKey: ConstructorParameters<typeof FakeGithub>[0]) {
    super(publicKey);
    this.install();
  }

  addRepo(fullName: string, id: number, defaultBranch = 'main'): void {
    this.repos.set(fullName, { id, default_branch: defaultBranch, private: true });
  }

  setPull(
    fullName: string,
    pull: Partial<FakePull> & { number: number; headSha: string; baseSha?: string },
  ): FakePull {
    const existing = this.pulls.get(`${fullName}#${pull.number}`);
    const value: FakePull = {
      number: pull.number,
      title: pull.title ?? existing?.title ?? `PR ${pull.number}`,
      state: pull.state ?? existing?.state ?? 'open',
      draft: pull.draft ?? existing?.draft ?? false,
      merged: pull.merged ?? existing?.merged ?? false,
      user: pull.user ?? existing?.user ?? { login: 'octo-dev', type: 'User' },
      head: { sha: pull.headSha, ref: pull.head?.ref ?? existing?.head.ref ?? 'feature' },
      base: {
        sha: pull.baseSha ?? existing?.base.sha ?? 'a'.repeat(40),
        ref: pull.base?.ref ?? existing?.base.ref ?? 'main',
      },
      updated_at: pull.updated_at ?? new Date().toISOString(),
      files: pull.files ?? existing?.files ?? [],
    };
    this.pulls.set(`${fullName}#${pull.number}`, value);
    return value;
  }

  /** A human-authored thread on a comment id (never resolved by the App). */
  addHumanThread(fullName: string, pull: number, commentId: number, login = 'octo-dev'): void {
    this.threads.push({
      id: `T_${commentId}`,
      repo: fullName,
      pull,
      isResolved: false,
      firstCommentId: commentId,
      author: login,
    });
  }

  reviewPosts(): FakeRequest[] {
    return this.requests.filter(
      (r) => r.method === 'POST' && /\/pulls\/\d+\/reviews$/.test(r.path),
    );
  }

  private id(): number {
    this.nextId += 1;
    return this.nextId;
  }

  private install(): void {
    const json = FakeGithub.json;
    const repoOf = (req: FakeRequest): string => `${req.params[0]}/${req.params[1]}`;

    this.routeMatch('GET', /^\/repos\/([^/]+)\/([^/]+)$/, (req, res) => {
      const repo = this.repos.get(repoOf(req));
      if (!repo) return json(res, 404, { message: 'Not Found' });
      json(res, 200, {
        id: repo.id,
        name: req.params[1],
        full_name: repoOf(req),
        owner: { login: req.params[0] },
        default_branch: repo.default_branch,
        private: repo.private,
        archived: false,
      });
    });

    this.routeMatch('GET', /^\/repos\/([^/]+)\/([^/]+)\/pulls$/, (req, res) => {
      if (!this.repos.has(repoOf(req))) return json(res, 404, { message: 'Not Found' });
      const open = [...this.pulls.entries()]
        .filter(([k, p]) => k.startsWith(`${repoOf(req)}#`) && p.state === 'open')
        .map(([, p]) => this.pullJson(p));
      const etag = `"${createHash('sha1').update(JSON.stringify(open)).digest('hex')}"`;
      if (req.headers['if-none-match'] === etag) {
        res.writeHead(304, { etag });
        return void res.end();
      }
      json(res, 200, open, { etag });
    });

    this.routeMatch('GET', /^\/repos\/([^/]+)\/([^/]+)\/pulls\/(\d+)$/, (req, res) => {
      const pull = this.pulls.get(`${repoOf(req)}#${req.params[2]}`);
      if (!pull) return json(res, 404, { message: 'Not Found' });
      json(res, 200, this.pullJson(pull));
    });

    this.routeMatch('GET', /^\/repos\/([^/]+)\/([^/]+)\/pulls\/(\d+)\/files$/, (req, res) => {
      const pull = this.pulls.get(`${repoOf(req)}#${req.params[2]}`);
      if (!pull) return json(res, 404, { message: 'Not Found' });
      const page = Number(req.query.get('page') ?? '1');
      const per = Number(req.query.get('per_page') ?? '30');
      json(res, 200, pull.files.slice((page - 1) * per, page * per));
    });

    this.routeMatch('GET', /^\/repos\/([^/]+)\/([^/]+)\/compare\/(.+)$/, (req, res) => {
      const files = this.compares.get(`${repoOf(req)}:${decodeURIComponent(req.params[2]!)}`);
      if (!files) return json(res, 404, { message: 'Not Found' });
      json(res, 200, { files: files.map((filename) => ({ filename, status: 'modified' })) });
    });

    this.routeMatch(
      'POST',
      /^\/repos\/([^/]+)\/([^/]+)\/pulls\/(\d+)\/reviews$/,
      async (req, res) => this.postReview(req, res, repoOf(req), Number(req.params[2])),
    );

    this.routeMatch('GET', /^\/repos\/([^/]+)\/([^/]+)\/pulls\/(\d+)\/reviews$/, (req, res) => {
      const page = Number(req.query.get('page') ?? '1');
      const per = Number(req.query.get('per_page') ?? '30');
      const list = this.reviews
        .filter((r) => r.repo === repoOf(req) && r.pull === Number(req.params[2]))
        .map((r) => ({ id: r.id, body: r.body, commit_id: r.commit_id }));
      json(res, 200, list.slice((page - 1) * per, page * per));
    });

    this.routeMatch(
      'GET',
      /^\/repos\/([^/]+)\/([^/]+)\/pulls\/(\d+)\/reviews\/(\d+)\/comments$/,
      (req, res) => {
        const review = this.reviews.find((r) => r.id === Number(req.params[3]));
        if (!review) return json(res, 404, { message: 'Not Found' });
        json(
          res,
          200,
          review.comments.map((c) => ({ id: c.id, body: c.body, path: c.path })),
        );
      },
    );

    this.routeMatch('POST', /^\/repos\/([^/]+)\/([^/]+)\/check-runs$/, (req, res) => {
      const id = this.id();
      this.checkRuns.push({ id, repo: repoOf(req), body: req.body as Record<string, unknown> });
      json(res, 201, { id });
    });
    this.routeMatch('PATCH', /^\/repos\/([^/]+)\/([^/]+)\/check-runs\/(\d+)$/, (req, res) => {
      const run = this.checkRuns.find((c) => c.id === Number(req.params[2]));
      if (!run) return json(res, 404, { message: 'Not Found' });
      run.body = { ...run.body, ...(req.body as Record<string, unknown>) };
      json(res, 200, { id: run.id });
    });

    this.route('POST', '/graphql', (req, res) => this.graphql(req, res));
  }

  private pullJson(p: FakePull): Record<string, unknown> {
    return {
      number: p.number,
      title: p.title,
      state: p.state,
      draft: p.draft,
      merged: p.merged,
      user: p.user,
      head: p.head,
      base: p.base,
      labels: [],
      updated_at: p.updated_at,
      body: 'never stored',
    };
  }

  private async postReview(
    req: FakeRequest,
    res: ServerResponse,
    repo: string,
    pull: number,
  ): Promise<void> {
    const body = req.body as {
      body: string;
      event: string;
      commit_id: string;
      comments?: { path: string; line: number; side: string; body: string }[];
    };
    if (this.reviewPostDelayMs) await new Promise((r) => setTimeout(r, this.reviewPostDelayMs));
    const fault = this.faults.reviewPost;
    if (fault === 422 && (body.comments?.length ?? 0) > 0) {
      return FakeGithub.json(res, 422, {
        message: 'Unprocessable Entity',
        errors: ['Line must be part of the diff'],
      });
    }
    if (typeof fault === 'number' && fault !== 422) {
      return FakeGithub.json(res, fault, { message: 'failure' });
    }
    const review: FakeReview = {
      id: this.id(),
      pull,
      repo,
      body: body.body,
      event: body.event,
      commit_id: body.commit_id,
      comments: (body.comments ?? []).map((c) => ({ id: this.id(), ...c })),
    };
    this.reviews.push(review);
    for (const c of review.comments) {
      this.threads.push({
        id: `T_${c.id}`,
        repo,
        pull,
        isResolved: false,
        firstCommentId: c.id,
        author: this.botLogin,
      });
    }
    if (fault === 'drop_after_write') {
      this.faults.reviewPost = undefined;
      res.socket?.destroy();
      return;
    }
    FakeGithub.json(res, 200, {
      id: review.id,
      body: review.body,
      html_url: `https://example/${review.id}`,
    });
  }

  private graphql(req: FakeRequest, res: ServerResponse): void {
    if (this.faults.graphql) {
      return FakeGithub.json(res, this.faults.graphql, { message: 'failure' });
    }
    const { query, variables } = req.body as { query: string; variables: Record<string, unknown> };
    if (query.includes('resolveReviewThread')) {
      const thread = this.threads.find((t) => t.id === variables.threadId);
      if (!thread) return FakeGithub.json(res, 200, { errors: [{ message: 'not found' }] });
      thread.isResolved = true;
      this.resolvedThreadIds.push(thread.id);
      return FakeGithub.json(res, 200, {
        data: { resolveReviewThread: { thread: { id: thread.id, isResolved: true } } },
      });
    }
    const repo = `${String(variables.owner)}/${String(variables.name)}`;
    const nodes = this.threads
      .filter((t) => t.repo === repo && t.pull === variables.number)
      .map((t) => ({
        id: t.id,
        isResolved: t.isResolved,
        comments: { nodes: [{ databaseId: t.firstCommentId, author: { login: t.author } }] },
      }));
    FakeGithub.json(res, 200, {
      data: {
        repository: {
          pullRequest: {
            reviewThreads: { pageInfo: { hasNextPage: false, endCursor: null }, nodes },
          },
        },
      },
    });
  }
}
