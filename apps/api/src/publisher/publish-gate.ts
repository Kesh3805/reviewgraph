import { Injectable } from '@nestjs/common';
import { sql } from 'kysely';
import { incCounter, recordHistogram } from '../common/metrics';
import type { Tx } from '../db/tx';

/** Lock wait of the publish side; a timeout fails the job, which retries with backoff. */
export const PUBLISH_LOCK_TIMEOUT = '20s';

export type GateSkipReason =
  | 'not_found'
  | 'superseded'
  | 'cancelled'
  | 'head_moved'
  | 'pr_closed'
  | 'repository_disabled'
  | 'not_publishing';

export interface GateRun {
  id: string;
  state: string;
  headSha: string;
  pullRequestId: string;
}

export type GateResult =
  { proceed: true; run: GateRun } | { proceed: false; reason: GateSkipReason; run?: GateRun };

/**
 * Publish-time gate (SUP-003). Inside the publish transaction it locks the pull request row and
 * then the run row (always this order, the same as SUP-001, so the two never deadlock) and
 * asserts that the run is still PUBLISHING, the PR head is still the run's head, the PR is open
 * and the repository is enabled. The caller posts the review while holding these locks, so a
 * supersession for a new head waits for the post (and, after it, sees a COMPLETED run), and a
 * publish for an obsolete head sees the moved head and posts nothing.
 */
@Injectable()
export class PublishGate {
  async acquire(trx: Tx, runId: string): Promise<GateResult> {
    await sql`select set_config('lock_timeout', ${PUBLISH_LOCK_TIMEOUT}, true)`.execute(trx);
    const started = performance.now();
    const pr = await trx
      .selectFrom('pull_requests as pr')
      .innerJoin('repositories as r', 'r.id', 'pr.repository_id')
      .innerJoin('provider_installations as i', 'i.id', 'r.installation_id')
      .leftJoin('repository_settings as s', 's.repository_id', 'r.id')
      .select([
        'pr.id',
        'pr.head_sha',
        'pr.state',
        'r.enabled',
        'r.access_state',
        'i.state as installation_state',
        's.enabled as settings_enabled',
      ])
      .where(
        'pr.id',
        '=',
        trx.selectFrom('review_runs').select('pull_request_id').where('id', '=', runId),
      )
      .forUpdate('pr')
      .executeTakeFirst();
    const runRow = await trx
      .selectFrom('review_runs')
      .select(['id', 'state', 'head_sha', 'pull_request_id'])
      .where('id', '=', runId)
      .forUpdate()
      .executeTakeFirst();
    recordHistogram('publish_gate_lock_wait_seconds', (performance.now() - started) / 1000);

    if (!pr || !runRow) return this.skip('not_found');
    const run: GateRun = {
      id: runRow.id,
      state: runRow.state,
      headSha: runRow.head_sha,
      pullRequestId: runRow.pull_request_id,
    };
    if (run.state === 'SUPERSEDED') return this.skip('superseded', run);
    if (run.state === 'CANCELLED') return this.skip('cancelled', run);
    if (run.state !== 'PUBLISHING') return this.skip('not_publishing', run);
    if (pr.head_sha !== run.headSha) return this.skip('head_moved', run);
    if (pr.state !== 'open') return this.skip('pr_closed', run);
    if (
      !pr.enabled ||
      pr.access_state !== 'active' ||
      pr.installation_state !== 'active' ||
      pr.settings_enabled === false
    ) {
      return this.skip('repository_disabled', run);
    }
    return { proceed: true, run };
  }

  private skip(reason: GateSkipReason, run?: GateRun): GateResult {
    incCounter('publish_gate_skips_total', { reason });
    return run ? { proceed: false, reason, run } : { proceed: false, reason };
  }
}
