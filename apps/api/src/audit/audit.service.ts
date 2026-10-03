import { Global, Injectable, Module } from '@nestjs/common';
import type { Tx } from '../db/tx';

export type AuditActorType = 'user' | 'service' | 'system';
export type AuditOutcome = 'success' | 'denied' | 'failure';

export interface AuditEvent {
  organizationId: string;
  repositoryId?: string;
  actor: { type: AuditActorType; id?: string };
  /** Dotted action name, for example `repository.settings.updated`. */
  action: string;
  targetType: string;
  targetId?: string;
  outcome?: AuditOutcome;
  /** Ids, enum values and before/after of non-secret fields only. */
  metadata?: Record<string, unknown>;
  requestId?: string;
}

/**
 * Baseline audit writer (API-008). Rows are written inside the transaction of the audited change,
 * so a change cannot succeed without its audit row. The hash chain, the typed event catalog and
 * the read API are SEC-008.
 */
@Injectable()
export class AuditService {
  async record(trx: Tx, event: AuditEvent): Promise<void> {
    await trx
      .insertInto('audit_log')
      .values({
        organization_id: event.organizationId,
        repository_id: event.repositoryId ?? null,
        actor_type: event.actor.type,
        actor_id: event.actor.id ?? null,
        action: event.action,
        target_type: event.targetType,
        target_id: event.targetId ?? null,
        outcome: event.outcome ?? 'success',
        metadata: JSON.stringify(event.metadata ?? {}),
        request_id: event.requestId ?? null,
      })
      .execute();
  }
}

@Global()
@Module({ providers: [AuditService], exports: [AuditService] })
export class AuditModule {}
