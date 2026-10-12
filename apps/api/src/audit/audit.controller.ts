import { Controller, Get, Param, Query } from '@nestjs/common';
import { ApiOkResponse, ApiTags } from '@nestjs/swagger';
import { sql } from 'kysely';
import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';
import { decodeTimeCursor, pageOf } from '../common/cursor';
import { DbService } from '../db/db.module';
import { Tenant, type RequestTenant } from '../tenancy/request-context';
import { RequireRole } from '../tenancy/roles.decorator';
import { AuditService } from './audit.service';

export const AuditQuerySchema = z.object({
  action: z.string().max(100).optional(),
  actor: z.string().max(200).optional(),
  from: z.iso.datetime({ offset: true }).optional(),
  to: z.iso.datetime({ offset: true }).optional(),
  cursor: z.string().max(500).optional(),
  limit: z.coerce.number().int().min(1).max(200).default(50),
});

export const AuditEntrySchema = z.object({
  id: z.string().uuid(),
  occurred_at: z.string(),
  repository_id: z.string().uuid().nullable(),
  actor_type: z.enum(['user', 'service', 'system']),
  actor_id: z.string().nullable(),
  action: z.string(),
  target_type: z.string(),
  target_id: z.string().nullable(),
  outcome: z.enum(['success', 'denied', 'failure']),
  metadata: z.record(z.string(), z.unknown()),
  request_id: z.string().nullable(),
  trace_id: z.string().nullable(),
  chain_seq: z.number().int().nullable(),
});

export const AuditPageSchema = z.object({
  items: z.array(AuditEntrySchema),
  next_cursor: z.string().nullable(),
});

export const AuditVerifySchema = z.object({
  ok: z.boolean(),
  checked: z.number().int(),
  first_invalid_id: z.string().uuid().nullable(),
});

class AuditQueryDto extends createZodDto(AuditQuerySchema) {}
class AuditPageDto extends createZodDto(AuditPageSchema) {}
class AuditVerifyDto extends createZodDto(AuditVerifySchema) {}

type AuditEntry = z.infer<typeof AuditEntrySchema>;

/** Audit log read API (SEC-008): organization admins only, tenant scoped by guard and RLS. */
@ApiTags('audit')
@Controller('organizations/:organizationId/audit')
export class AuditController {
  constructor(
    private readonly dbs: DbService,
    private readonly audit: AuditService,
  ) {}

  @Get()
  @RequireRole('admin')
  @ApiOkResponse({ type: AuditPageDto })
  async list(
    @Param('organizationId') organizationId: string,
    @Query() query: AuditQueryDto,
    @Tenant() tenant: RequestTenant,
  ): Promise<{ items: AuditEntry[]; next_cursor: string | null }> {
    const after = decodeTimeCursor(query.cursor);
    const rows = await this.dbs.withTx(tenant.organizationId, (trx) => {
      let q = trx
        .selectFrom('audit_log')
        .select([
          'id',
          'occurred_at',
          'repository_id',
          'actor_type',
          'actor_id',
          'action',
          'target_type',
          'target_id',
          'outcome',
          'metadata',
          'request_id',
          'trace_id',
          'chain_seq',
          sql<string>`to_char(occurred_at at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')`.as(
            'cursor_ts',
          ),
        ])
        .where('organization_id', '=', organizationId);
      if (query.action) q = q.where('action', '=', query.action);
      if (query.actor) q = q.where('actor_id', '=', query.actor);
      if (query.from) q = q.where('occurred_at', '>=', new Date(query.from));
      if (query.to) q = q.where('occurred_at', '<', new Date(query.to));
      if (after) {
        q = q.where(
          sql<boolean>`(occurred_at, id) < (${after.createdAt}::timestamptz, ${after.id}::uuid)`,
        );
      }
      return q
        .orderBy('occurred_at', 'desc')
        .orderBy('id', 'desc')
        .limit(query.limit + 1)
        .execute();
    });
    const { page, next_cursor } = pageOf(rows, query.limit);
    return {
      items: page.map((r) => ({
        id: r.id,
        occurred_at: r.occurred_at.toISOString(),
        repository_id: r.repository_id,
        actor_type: r.actor_type as AuditEntry['actor_type'],
        actor_id: r.actor_id,
        action: r.action,
        target_type: r.target_type,
        target_id: r.target_id,
        outcome: r.outcome as AuditEntry['outcome'],
        metadata: (r.metadata ?? {}) as Record<string, unknown>,
        request_id: r.request_id,
        trace_id: r.trace_id,
        chain_seq: r.chain_seq === null ? null : Number(r.chain_seq),
      })),
      next_cursor,
    };
  }

  /** Recomputes the organization's hash chain and reports the first row that does not match. */
  @Get('verify')
  @RequireRole('admin')
  @ApiOkResponse({ type: AuditVerifyDto })
  verify(
    @Param('organizationId') _organizationId: string,
    @Tenant() tenant: RequestTenant,
  ): Promise<z.infer<typeof AuditVerifySchema>> {
    return this.audit.verify(tenant.organizationId);
  }
}
