import { ProviderError, type PrRef } from '../ports';
import { githubCall } from './api-call';
import type { GithubOctokit } from './octokit.factory';

/** Review threads per GraphQL page; at most this many pages are read. */
export const THREADS_PER_PAGE = 100;
const MAX_THREAD_PAGES = 10;

export const REVIEW_THREADS_QUERY = `query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: ${THREADS_PER_PAGE}, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { id isResolved comments(first: 1) { nodes { databaseId author { login } } } }
      }
    }
  }
}`;

export const RESOLVE_THREAD_MUTATION = `mutation($threadId: ID!) {
  resolveReviewThread(input: { threadId: $threadId }) { thread { id isResolved } }
}`;

export interface ReviewThread {
  id: string;
  isResolved: boolean;
  /** `databaseId` of the thread's first comment (the REST comment id). */
  firstCommentId?: string;
  firstCommentAuthor?: string;
}

interface GraphqlResponse<T> {
  data?: T;
  errors?: { message?: string }[];
}

interface ThreadsData {
  repository?: {
    pullRequest?: {
      reviewThreads?: {
        pageInfo?: { hasNextPage?: boolean; endCursor?: string | null };
        nodes?: {
          id: string;
          isResolved: boolean;
          comments?: { nodes?: { databaseId?: number; author?: { login?: string } | null }[] };
        }[];
      };
    };
  };
}

async function graphql<T>(
  octokit: GithubOctokit,
  operation: string,
  query: string,
  variables: Record<string, unknown>,
): Promise<T> {
  const { data } = await githubCall<GraphqlResponse<T>>(
    'POST /graphql',
    operation,
    async () =>
      (await octokit.request('POST /graphql', { query, variables })) as {
        status: number;
        data: GraphqlResponse<T>;
      },
  );
  if (data.errors?.length || !data.data) {
    // GraphQL reports failures with HTTP 200; the messages may quote input, so they are dropped.
    throw new ProviderError('invalid', `${operation} failed (graphql error)`);
  }
  return data.data;
}

/** Lists the review threads of a pull request (paged, bounded). */
export async function listReviewThreads(
  octokit: GithubOctokit,
  ref: PrRef,
): Promise<ReviewThread[]> {
  const threads: ReviewThread[] = [];
  let after: string | null = null;
  for (let page = 0; page < MAX_THREAD_PAGES; page++) {
    const data: ThreadsData = await graphql<ThreadsData>(
      octokit,
      'review thread listing',
      REVIEW_THREADS_QUERY,
      { owner: ref.owner, name: ref.name, number: ref.number, after },
    );
    const conn = data.repository?.pullRequest?.reviewThreads;
    for (const node of conn?.nodes ?? []) {
      const first = node.comments?.nodes?.[0];
      threads.push({
        id: node.id,
        isResolved: node.isResolved,
        ...(first?.databaseId !== undefined ? { firstCommentId: String(first.databaseId) } : {}),
        ...(first?.author?.login ? { firstCommentAuthor: first.author.login } : {}),
      });
    }
    if (!conn?.pageInfo?.hasNextPage || !conn.pageInfo.endCursor) break;
    after = conn.pageInfo.endCursor;
  }
  return threads;
}

export async function resolveReviewThread(octokit: GithubOctokit, threadId: string): Promise<void> {
  await graphql(octokit, 'review thread resolution', RESOLVE_THREAD_MUTATION, { threadId });
}

/**
 * GraphQL reports an App's bot as `slug` while REST says `slug[bot]`: both forms are accepted.
 * Without a configured slug nothing matches (fail closed: no thread is touched).
 */
export function isAppAuthor(login: string | undefined, appSlug: string | undefined): boolean {
  if (!login || !appSlug) return false;
  const l = login.toLowerCase();
  const s = appSlug.toLowerCase();
  return l === s || l === `${s}[bot]`;
}
