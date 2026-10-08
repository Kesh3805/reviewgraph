import { createHash } from 'node:crypto';
import { HttpStatus, Inject, Injectable, NotFoundException } from '@nestjs/common';
import { SpanKind, SpanStatusCode, trace } from '@opentelemetry/api';
import type { Redis } from 'ioredis';
import { incCounter } from '../common/metrics';
import { ProblemException } from '../common/problem.filter';
import { REDIS } from '../common/redis.module';
import { TRACER_NAME } from '../telemetry/tracer.service';
import type {
  GraphResponse,
  NeighborsQuery,
  PathQuery,
  SourceExcerpt,
  SourceQuery,
  SubgraphRequest,
  SymbolSearchQuery,
} from './dto/graph.dto';
import { EngineClient, type EngineScope } from './engine.client';
import { GRAPH_SCOPE, type GraphScope } from './graph-scope';
import { redactExcerpt } from './redact';

/** Snapshots are immutable, so a query result for one can be cached safely. */
export const GRAPH_CACHE_TTL_SECONDS = 300;

export const graphCacheKey = (repositoryId: string, snapshotId: string, query: unknown): string =>
  `rg:gq:${repositoryId}:${snapshotId}:${createHash('sha256').update(JSON.stringify(query)).digest('hex')}`;

export interface GraphCaller {
  organizationId: string;
  userId: string;
  requestId?: string;
}

/**
 * The authorized graph proxy (API-011, target-architecture section 5 `graph` module). The
 * tenant scope (organization and repository) is injected here, from the route and the resolved
 * tenant, and never taken from the client.
 */
@Injectable()
export class GraphService {
  constructor(
    private readonly engine: EngineClient,
    @Inject(GRAPH_SCOPE) private readonly scope: GraphScope,
    @Inject(REDIS) private readonly redis: Redis,
  ) {}

  searchSymbols(caller: GraphCaller, repositoryId: string, q: SymbolSearchQuery) {
    return this.cachedQuery(caller, repositoryId, q.snapshot, 'symbols', q, (s, snap) =>
      this.engine.searchSymbols(s, snap, { q: q.q, kind: q.kind, limit: q.limit }),
    );
  }

  symbol(caller: GraphCaller, repositoryId: string, key: string, snapshot?: string) {
    return this.cachedQuery(caller, repositoryId, snapshot, 'symbol', { key }, (s, snap) =>
      this.engine.symbol(s, snap, key),
    );
  }

  neighbors(caller: GraphCaller, repositoryId: string, key: string, q: NeighborsQuery) {
    return this.cachedQuery(
      caller,
      repositoryId,
      q.snapshot,
      'neighbors',
      { key, ...q },
      (s, snap) =>
        this.engine.neighbors(s, snap, key, {
          dir: q.dir,
          kinds: q.kinds,
          minConfidence: q.min_confidence,
        }),
    );
  }

  subgraph(caller: GraphCaller, repositoryId: string, body: SubgraphRequest) {
    return this.cachedQuery(caller, repositoryId, body.snapshot, 'subgraph', body, (s, snap) =>
      this.engine.subgraph(s, snap, {
        seeds: body.seeds,
        depth: body.depth,
        kinds: body.kinds,
        maxNodes: body.max_nodes,
      }),
    );
  }

  path(caller: GraphCaller, repositoryId: string, q: PathQuery) {
    return this.cachedQuery(caller, repositoryId, q.snapshot, 'path', q, (s, snap) =>
      this.engine.path(s, snap, { from: q.from, to: q.to, maxDepth: q.max_depth }),
    );
  }

  /** Impact of a symbol in a review (the run's head graph); scoped to the run's repository. */
  async impact(caller: GraphCaller, reviewRunId: string, key: string): Promise<GraphResponse> {
    const repositoryId = await this.scope.reviewRepository(caller.organizationId, reviewRunId);
    if (!repositoryId) throw new NotFoundException();
    const scope = { organizationId: caller.organizationId, repositoryId };
    const cacheSnapshot = `review-${reviewRunId}`;
    return this.withCache('impact', repositoryId, cacheSnapshot, { key }, () =>
      this.proxied('impact', () => this.engine.impact(scope, reviewRunId, key)),
    );
  }

  /**
   * A redacted source excerpt (at most 200 lines). The engine redacts and the proxy redacts
   * again; excerpts are never cached, and every request is audited.
   */
  async source(caller: GraphCaller, repositoryId: string, q: SourceQuery): Promise<SourceExcerpt> {
    const scope: EngineScope = { organizationId: caller.organizationId, repositoryId };
    const snapshotId = await this.resolveSnapshot(scope, q.snapshot);
    await this.scope.recordSourceAccess({
      organizationId: caller.organizationId,
      repositoryId,
      userId: caller.userId,
      snapshotId,
      path: q.path,
      start: q.start,
      end: q.end,
      requestId: caller.requestId,
    });
    const excerpt = await this.proxied('source', () =>
      this.engine.source(scope, snapshotId, { path: q.path, start: q.start, end: q.end }),
    );
    incCounter('graph_queries_total', { route: 'source', cached: 'false' });
    const lines = excerpt.text.split('\n');
    const capped = lines.length > q.end - q.start + 1;
    const { text, redactions } = redactExcerpt(
      capped ? lines.slice(0, q.end - q.start + 1).join('\n') : excerpt.text,
    );
    return {
      snapshot_id: excerpt.snapshot_id ?? snapshotId,
      path: excerpt.path ?? q.path,
      start: excerpt.start ?? q.start,
      end: excerpt.end ?? q.end,
      text,
      redacted: redactions > 0,
      truncated: Boolean(excerpt.truncated) || capped,
    };
  }

  private async cachedQuery(
    caller: GraphCaller,
    repositoryId: string,
    requestedSnapshot: string | undefined,
    route: string,
    query: unknown,
    call: (scope: EngineScope, snapshotId: string) => Promise<GraphResponse>,
  ): Promise<GraphResponse> {
    const scope: EngineScope = { organizationId: caller.organizationId, repositoryId };
    const snapshotId = await this.resolveSnapshot(scope, requestedSnapshot);
    return this.withCache(route, repositoryId, snapshotId, { route, query }, () =>
      this.proxied(route, () => call(scope, snapshotId)),
    );
  }

  /** The requested snapshot after an ownership check, or the default-branch snapshot. */
  private async resolveSnapshot(
    scope: EngineScope,
    requested: string | undefined,
  ): Promise<string> {
    if (requested) {
      const owned = await this.scope.snapshotBelongsTo(
        scope.organizationId,
        scope.repositoryId,
        requested,
      );
      // Unknown and foreign snapshots are indistinguishable.
      if (!owned) throw new ProblemException(HttpStatus.NOT_FOUND, 'unknown snapshot');
      return requested;
    }
    const latest = await this.scope.defaultSnapshot(scope.organizationId, scope.repositoryId);
    if (!latest) {
      throw new ProblemException(HttpStatus.NOT_FOUND, 'the repository has no graph snapshot yet');
    }
    return latest;
  }

  private async withCache(
    route: string,
    repositoryId: string,
    snapshotId: string,
    query: unknown,
    load: () => Promise<GraphResponse>,
  ): Promise<GraphResponse> {
    const key = graphCacheKey(repositoryId, snapshotId, query);
    const hit = await this.redis.get(key).catch(() => null);
    if (hit) {
      incCounter('graph_queries_total', { route, cached: 'true' });
      return JSON.parse(hit) as GraphResponse;
    }
    const value = await load();
    incCounter('graph_queries_total', { route, cached: 'false' });
    // A cache write failure only costs the next query a round trip.
    await this.redis
      .set(key, JSON.stringify(value), 'EX', GRAPH_CACHE_TTL_SECONDS)
      .catch(() => undefined);
    return value;
  }

  private proxied<T>(route: string, call: () => Promise<T>): Promise<T> {
    return trace
      .getTracer(TRACER_NAME)
      .startActiveSpan(
        'engine_proxy',
        { kind: SpanKind.CLIENT, attributes: { route } },
        async (span) => {
          try {
            return await call();
          } catch (err) {
            span.setStatus({ code: SpanStatusCode.ERROR });
            throw err;
          } finally {
            span.end();
          }
        },
      );
  }
}
