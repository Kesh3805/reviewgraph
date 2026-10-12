import { Injectable } from '@nestjs/common';
import { sql } from 'kysely';
import { AuditService } from '../audit/audit.service';
import { DbService } from '../db/db.module';

/**
 * The database side of the graph proxy: which snapshot a query runs against, whether a snapshot
 * belongs to a repository, which repository a review run belongs to, and the source-access
 * audit (SEC-008). Only `ready` snapshots (GS-003 `snapshots`) are served; before the first
 * index a repository has none and graph queries answer 404.
 */
export interface GraphScope {
  /** The latest full snapshot of the default branch, or null before the first index. */
  defaultSnapshot(orgId: string, repositoryId: string): Promise<string | null>;
  /** True only when the snapshot exists and belongs to the repository (tenant scoped). */
  snapshotBelongsTo(orgId: string, repositoryId: string, snapshotId: string): Promise<boolean>;
  /** The repository of a review run, or null when unknown in this tenant. */
  reviewRepository(orgId: string, reviewRunId: string): Promise<string | null>;
  /** Audits a source excerpt request (who read which lines of which file). */
  recordSourceAccess(event: {
    organizationId: string;
    repositoryId: string;
    userId: string;
    snapshotId: string;
    path: string;
    start: number;
    end: number;
    requestId?: string;
  }): Promise<void>;
}
export const GRAPH_SCOPE = Symbol('GRAPH_SCOPE');

@Injectable()
export class PgGraphScope implements GraphScope {
  constructor(
    private readonly dbs: DbService,
    private readonly audit: AuditService,
  ) {}

  /** The latest ready full snapshot of the default branch (GS-003 `snapshots`). */
  async defaultSnapshot(orgId: string, repositoryId: string): Promise<string | null> {
    const row = await this.dbs.withTx(orgId, (trx) =>
      trx
        .selectFrom('snapshots')
        .select('id')
        .where('repository_id', '=', repositoryId)
        .where('kind', '=', 'full')
        .where('purpose', '=', 'default_branch')
        .where('status', '=', 'ready')
        .orderBy(sql`completed_at desc nulls last`)
        .orderBy('created_at', 'desc')
        .orderBy('id', 'desc')
        .executeTakeFirst(),
    );
    return row?.id ?? null;
  }

  /** Any snapshot of the repository in this tenant (RLS hides other tenants' snapshots). */
  async snapshotBelongsTo(
    orgId: string,
    repositoryId: string,
    snapshotId: string,
  ): Promise<boolean> {
    const row = await this.dbs.withTx(orgId, (trx) =>
      trx
        .selectFrom('snapshots')
        .select('id')
        .where('id', '=', snapshotId)
        .where('repository_id', '=', repositoryId)
        .where('status', '=', 'ready')
        .executeTakeFirst(),
    );
    return row !== undefined;
  }

  async reviewRepository(orgId: string, reviewRunId: string): Promise<string | null> {
    const row = await this.dbs.withTx(orgId, (trx) =>
      trx
        .selectFrom('review_runs')
        .select('repository_id')
        .where('id', '=', reviewRunId)
        .executeTakeFirst(),
    );
    return row?.repository_id ?? null;
  }

  async recordSourceAccess(event: Parameters<GraphScope['recordSourceAccess']>[0]): Promise<void> {
    await this.dbs.withTx(event.organizationId, (trx) =>
      this.audit.record(trx, {
        organizationId: event.organizationId,
        repositoryId: event.repositoryId,
        actor: { type: 'user', id: event.userId },
        action: 'source.excerpt.read',
        targetType: 'source',
        targetId: event.path,
        requestId: event.requestId,
        metadata: {
          snapshot_id: event.snapshotId,
          path: event.path,
          start: event.start,
          end: event.end,
        },
      }),
    );
  }
}
