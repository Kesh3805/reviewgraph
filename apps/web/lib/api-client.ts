import createClient, { type Middleware } from 'openapi-fetch';
import type { paths } from './api/schema';
import { withCsrf } from './csrf';

/** RFC 9457 problem+json body returned by the API. */
export interface Problem {
  type?: string;
  title?: string;
  status?: number;
  detail?: string;
  instance?: string;
  request_id?: string;
}

/** Any non-2xx API response, with the problem+json fields mapped onto the error. */
export class ApiError extends Error {
  readonly status: number;
  readonly problem: Problem;
  readonly requestId?: string;

  constructor(status: number, problem: Problem) {
    super(problem.detail ?? problem.title ?? `Request failed with status ${status}`);
    this.name = 'ApiError';
    this.status = status;
    this.problem = problem;
    this.requestId = problem.request_id;
  }

  get isUnauthorized(): boolean {
    return this.status === 401;
  }
}

/** Builds an `ApiError` from a failed response, tolerating non-JSON bodies. */
export async function toApiError(res: Response): Promise<ApiError> {
  let problem: Problem = { status: res.status, title: res.statusText };
  const type = res.headers.get('content-type') ?? '';
  if (type.includes('json')) {
    try {
      problem = { ...problem, ...((await res.clone().json()) as Problem) };
    } catch {
      // Keep the status-only problem.
    }
  }
  return new ApiError(res.status, problem);
}

/** Injects the CSRF header on mutations and turns error responses into `ApiError`. */
export const apiMiddleware: Middleware = {
  onRequest({ request }) {
    withCsrf(request.method, request.headers);
    return request;
  },
  async onResponse({ response }) {
    if (!response.ok) throw await toApiError(response);
    return response;
  },
};

/**
 * Typed client for the browser. `baseUrl` is empty so requests are same-origin and the
 * Next rewrite forwards `/api/*` to the API.
 */
export function createApiClient(baseUrl = '') {
  const client = createClient<paths>({ baseUrl, credentials: 'same-origin' });
  client.use(apiMiddleware);
  return client;
}

export const api = createApiClient();
