import { HttpStatus, Inject, Injectable, Logger } from '@nestjs/common';
import { ProblemException } from '../common/problem.filter';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { ServiceTokenService } from '../internal/internal.module';
import type { GraphResponse } from './dto/graph.dto';

export const ENGINE_TIMEOUT_MS = 5_000;

/** The tenant scope of an engine call. It is always derived server-side, never from the client. */
export interface EngineScope {
  organizationId: string;
  repositoryId: string;
}

/** Raw engine excerpt (API-013 `source` route): already redacted and capped by the engine. */
export interface EngineSourceExcerpt {
  snapshot_id: string;
  path: string;
  start: number;
  end: number;
  text: string;
  truncated?: boolean;
}

export interface NeighborsParams {
  dir: 'in' | 'out';
  kinds?: string[];
  minConfidence?: number;
}

export interface SubgraphParams {
  seeds: string[];
  depth: number;
  kinds?: string[];
  maxNodes: number;
}

/**
 * Typed client of the review-engine internal API (API-013, `/internal/v1`). Every call carries a
 * fresh service token (`aud=rg-engine`, `scope=graph:read`, `org`, `repo`) so the engine can check
 * that the repository in the path is the one the token was minted for. Node's built-in fetch
 * (undici) keeps connections alive; each call is bounded by a 5 s timeout.
 */
@Injectable()
export class EngineClient {
  private readonly logger = new Logger(EngineClient.name);
  private readonly base: string;

  constructor(
    @Inject(APP_CONFIG) config: AppConfig,
    private readonly tokens: ServiceTokenService,
  ) {
    this.base = new URL('/internal/v1/', config.ENGINE_INTERNAL_URL).toString();
  }

  searchSymbols(
    scope: EngineScope,
    snapshotId: string,
    params: { q?: string; kind?: string; limit: number },
  ): Promise<GraphResponse> {
    return this.get(scope, `${this.snap(scope, snapshotId)}/symbols`, {
      q: params.q,
      kind: params.kind,
      limit: String(params.limit),
    });
  }

  symbol(scope: EngineScope, snapshotId: string, key: string): Promise<GraphResponse> {
    return this.get(scope, `${this.snap(scope, snapshotId)}/symbols/${enc(key)}`);
  }

  neighbors(
    scope: EngineScope,
    snapshotId: string,
    key: string,
    params: NeighborsParams,
  ): Promise<GraphResponse> {
    return this.get(scope, `${this.snap(scope, snapshotId)}/symbols/${enc(key)}/neighbors`, {
      dir: params.dir,
      kinds: params.kinds?.join(','),
      min_confidence: params.minConfidence?.toString(),
    });
  }

  subgraph(scope: EngineScope, snapshotId: string, params: SubgraphParams): Promise<GraphResponse> {
    return this.request(scope, 'POST', `${this.snap(scope, snapshotId)}/subgraph`, undefined, {
      seeds: params.seeds,
      depth: params.depth,
      kinds: params.kinds,
      max_nodes: params.maxNodes,
    });
  }

  path(
    scope: EngineScope,
    snapshotId: string,
    params: { from: string; to: string; maxDepth: number },
  ): Promise<GraphResponse> {
    return this.get(scope, `${this.snap(scope, snapshotId)}/path`, {
      from: params.from,
      to: params.to,
      max_depth: String(params.maxDepth),
    });
  }

  impact(scope: EngineScope, reviewRunId: string, key: string): Promise<GraphResponse> {
    return this.get(scope, `reviews/${enc(reviewRunId)}/impact/${enc(key)}`);
  }

  source(
    scope: EngineScope,
    snapshotId: string,
    params: { path: string; start: number; end: number },
  ): Promise<EngineSourceExcerpt> {
    return this.get(scope, `${this.snap(scope, snapshotId)}/source`, {
      path: params.path,
      start: String(params.start),
      end: String(params.end),
    });
  }

  private snap(scope: EngineScope, snapshotId: string): string {
    return `repos/${enc(scope.repositoryId)}/snapshots/${enc(snapshotId)}`;
  }

  private get<T>(
    scope: EngineScope,
    path: string,
    query?: Record<string, string | undefined>,
  ): Promise<T> {
    return this.request<T>(scope, 'GET', path, query);
  }

  private async request<T>(
    scope: EngineScope,
    method: 'GET' | 'POST',
    path: string,
    query?: Record<string, string | undefined>,
    body?: unknown,
  ): Promise<T> {
    const url = new URL(path, this.base);
    for (const [k, v] of Object.entries(query ?? {}))
      if (v !== undefined) url.searchParams.set(k, v);
    const token = await this.tokens.issue({
      aud: 'rg-engine',
      scope: ['graph:read'],
      org: scope.organizationId,
      repo: scope.repositoryId,
    });
    let res: Response;
    try {
      res = await fetch(url, {
        method,
        headers: {
          authorization: `Bearer ${token}`,
          accept: 'application/json',
          ...(body === undefined ? {} : { 'content-type': 'application/json' }),
        },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(ENGINE_TIMEOUT_MS),
      });
    } catch (err) {
      this.logger.warn(`engine ${method} ${url.pathname} failed: ${String(err)}`);
      throw engineUnavailable();
    }
    if (res.ok) return (await res.json()) as T;
    const detail = await res.text().catch(() => '');
    if (res.status === 404)
      throw new ProblemException(HttpStatus.NOT_FOUND, 'not found in the graph');
    if (res.status === 400 || res.status === 422) {
      throw new ProblemException(
        HttpStatus.BAD_REQUEST,
        problemDetail(detail) ?? 'invalid graph query',
      );
    }
    if (res.status === 503) {
      // The engine is loading the snapshot: let the client retry.
      const retryAfter = res.headers.get('retry-after') ?? '2';
      throw new ProblemException(
        HttpStatus.SERVICE_UNAVAILABLE,
        'the graph is loading',
        { retry_after_seconds: Number(retryAfter) || 2 },
        { 'Retry-After': retryAfter },
      );
    }
    // 401/403 mean a misconfigured service key; anything else is an engine fault.
    this.logger.error(`engine ${method} ${url.pathname} answered ${res.status}`);
    throw engineUnavailable();
  }
}

const enc = (segment: string): string => encodeURIComponent(segment);

function engineUnavailable(): ProblemException {
  return new ProblemException(HttpStatus.SERVICE_UNAVAILABLE, 'the review engine is unavailable');
}

function problemDetail(body: string): string | undefined {
  try {
    const parsed = JSON.parse(body) as { detail?: unknown };
    return typeof parsed.detail === 'string' ? parsed.detail.slice(0, 500) : undefined;
  } catch {
    return undefined;
  }
}
