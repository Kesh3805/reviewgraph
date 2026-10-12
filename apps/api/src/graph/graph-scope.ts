import { Injectable } from '@nestjs/common';
import { AuditService } from '../audit/audit.service';
import { DbService } from '../db/db.module';

/**
 * The database side of the graph proxy: which snapshot a query runs against, whether a snapshot
 * belongs to a repository, which repository a review run belongs to, and the source-access
 * audit (SEC-008). Graph snapshots (`snapshots`, GS-005) do not exist in the schema yet: until
 * they do, a repository has no default snapshot and no snapshot id validates, so graph queries
 * answer 404 and a client-supplied snapshot is rejected.
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

  defaultSnapshot(): Promise<string | null> {
    // GS-005 replaces this with: latest `snapshots` row (kind='full', purpose='default_branch',
    // status='ready') of the repository.
    return Promise.resolve(null);
  }

  snapshotBelongsTo(): Promise<boolean> {
    return Promise.resolve(false);
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
