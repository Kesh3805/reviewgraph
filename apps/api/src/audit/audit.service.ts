import { Injectable, Logger } from '@nestjs/common';
import { trace } from '@opentelemetry/api';
import { sql } from 'kysely';
import { incCounter } from '../common/metrics';
import { redactValue } from '../common/redact';
import { DbService } from '../db/db.module';
import type { Tx } from '../db/tx';
import type { AuditEvent } from './audit.types';

export type { AuditActorType, AuditEvent, AuditOutcome } from './audit.types';

/**
 * Audit writer (API-008 baseline, SEC-008). Rows are written inside the transaction of the
 * audited change, so a change cannot succeed without its audit row and a failed audit write
 * rolls the change back. The database chains every row into a per-organization hash chain
 * (`rg_audit_chain` trigger) and rejects UPDATE and DELETE.
 */
@Injectable()
export class AuditService {
  private readonly logger = new Logger(AuditService.name);

  constructor(private readonly dbs: DbService) {}

  /** Writes the event in the caller's transaction. Returns false for a deduplicated event. */
  async record(trx: Tx, event: AuditEvent): Promise<boolean> {
    const spanCtx = trace.getActiveSpan()?.spanContext();
    try {
      let insert = trx.insertInto('audit_log').values({
        organization_id: event.organizationId,
        repository_id: event.repositoryId ?? null,
        actor_type: event.actor.type,
        actor_id: event.actor.id ?? null,
        action: event.action,
        target_type: event.targetType,
        target_id: event.targetId ?? null,
        outcome: event.outcome ?? 'success',
        metadata: JSON.stringify(redactValue(event.metadata ?? {})),
        request_id: event.requestId ?? null,
        trace_id: spanCtx && spanCtx.traceId !== '0'.repeat(32) ? spanCtx.traceId : null,
        dedupe_key: event.dedupeKey ?? null,
      });
      if (event.dedupeKey) {
        insert = insert.onConflict((oc) =>
          oc
            .columns(['organization_id', 'dedupe_key'])
            .where('dedupe_key', 'is not', null)
            .doNothing(),
        );
      }
      const written = (await insert.returning('id').executeTakeFirst()) !== undefined;
      if (written) {
        incCounter('audit_events_total', {
          action: event.action,
          outcome: event.outcome ?? 'success',
        });
      }
      return written;
    } catch (err) {
      incCounter('audit_write_failures_total');
      throw err;
    }
  }

  /**
   * Writes the event in its own short transaction, for events that must survive the rollback of
   * the request that caused them (access denials, rejected replays). Best effort: a failure is
   * logged and counted, never thrown.
   */
  async recordStandalone(event: AuditEvent): Promise<void> {
    try {
      await this.dbs.withTx(event.organizationId, (trx) => this.record(trx, event));
    } catch (err) {
      this.logger.error(`audit write failed for ${event.action}: ${String(err)}`);
    }
  }

  /** Recomputes the organization's hash chain (RLS limits it to the caller's tenant). */
  async verify(
    orgId: string,
  ): Promise<{ ok: boolean; checked: number; first_invalid_id: string | null }> {
    const { rows } = await this.dbs.withTx(orgId, (trx) =>
      sql<{
        ok: boolean;
        checked: string;
        first_invalid_id: string | null;
      }>`select * from rg_audit_verify(${orgId}::uuid)`.execute(trx),
    );
    const row = rows[0] ?? { ok: true, checked: '0', first_invalid_id: null };
    if (!row.ok) incCounter('audit_chain_verify_failures_total');
    return { ok: row.ok, checked: Number(row.checked), first_invalid_id: row.first_invalid_id };
  }
}
