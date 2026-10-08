import { Injectable } from '@nestjs/common';
import { sql } from 'kysely';
import { incCounter } from '../common/metrics';
import { DbService } from '../db/db.module';
import type { ProviderPullRequest } from '../providers/ports';

export interface PullRequestTarget {
  organizationId: string;
  repositoryId: string;
}

export interface PullRequestUpsertResult {
  pullRequestId: string;
  /** False when the stored row was newer (an out-of-order older event) and was left alone. */
  applied: boolean;
  /** True when the row did not exist before (its head was set by this insert). */
  created: boolean;
}

/** `ProviderPullRequest.updatedAt` as a Date, or null when absent or unparsable. */
export function providerUpdatedAt(pr: Pick<ProviderPullRequest, 'updatedAt'>): Date | null {
  if (!pr.updatedAt) return null;
  const at = new Date(pr.updatedAt);
  return Number.isNaN(at.getTime()) ? null : at;
}

/**
 * Upserts `pull_requests` from authoritative provider reads (GH-005). The update is guarded by
 * the provider's `updated_at`: an out-of-order older event never overwrites a newer row. The
 * head and base SHAs are written only on insert; head changes go through SUP-001, which moves
 * the head and supersedes older runs in one transaction. The PR body is never stored.
 */
@Injectable()
export class PullRequestSyncService {
  constructor(private readonly dbs: DbService) {}

  async upsert(
    target: PullRequestTarget,
    pr: ProviderPullRequest,
  ): Promise<PullRequestUpsertResult> {
    const updatedAt = providerUpdatedAt(pr);
    return this.dbs.withTx(target.organizationId, async (trx) => {
      const row = await trx
        .insertInto('pull_requests')
        .values({
          organization_id: target.organizationId,
          repository_id: target.repositoryId,
          provider_number: pr.ref.number,
          title: pr.title.slice(0, 512),
          author_login: pr.author.login,
          base_ref: pr.baseRef,
          head_ref: pr.headRef,
          base_sha: pr.baseSha,
          head_sha: pr.headSha,
          state: pr.state,
          draft: pr.draft,
          provider_updated_at: updatedAt,
        })
        .onConflict((oc) =>
          oc
            .columns(['repository_id', 'provider_number'])
            .doUpdateSet((eb) => ({
              title: eb.ref('excluded.title'),
              author_login: eb.ref('excluded.author_login'),
              base_ref: eb.ref('excluded.base_ref'),
              head_ref: eb.ref('excluded.head_ref'),
              state: eb.ref('excluded.state'),
              draft: eb.ref('excluded.draft'),
              provider_updated_at: sql<Date>`greatest(pull_requests.provider_updated_at, excluded.provider_updated_at)`,
            }))
            .where(
              sql<boolean>`pull_requests.provider_updated_at is null
                or excluded.provider_updated_at is null
                or pull_requests.provider_updated_at <= excluded.provider_updated_at`,
            ),
        )
        // xmax = 0 only for a freshly inserted tuple.
        .returning(['id', sql<boolean>`(xmax = 0)`.as('inserted')])
        .executeTakeFirst();
      if (row) {
        incCounter('pull_request_sync_total', { outcome: row.inserted ? 'inserted' : 'updated' });
        return { pullRequestId: row.id, applied: true, created: row.inserted };
      }
      const existing = await trx
        .selectFrom('pull_requests')
        .select('id')
        .where('repository_id', '=', target.repositoryId)
        .where('provider_number', '=', pr.ref.number)
        .executeTakeFirstOrThrow();
      incCounter('pull_request_sync_total', { outcome: 'stale_ignored' });
      return { pullRequestId: existing.id, applied: false, created: false };
    });
  }
}
