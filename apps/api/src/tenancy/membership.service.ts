import { Injectable } from '@nestjs/common';
import { sql } from 'kysely';
import { DbService } from '../db/db.module';
import type { MembershipRole } from './request-context';

export type ResourceKind =
  'repository' | 'pull_request' | 'review' | 'finding' | 'installation' | 'organization';

export interface UserMembership {
  organizationId: string;
  slug: string;
  displayName: string;
  role: MembershipRole;
}

/**
 * Pre-tenant lookups (which organization owns this id, what role does this user hold there).
 * They go through narrow SECURITY DEFINER functions (see the RLS migration), so the API role
 * never needs to read other tenants rows.
 */
@Injectable()
export class MembershipService {
  constructor(private readonly dbs: DbService) {}

  /** Organization owning a resource, or null when the id is unknown. */
  async resolveOrg(kind: ResourceKind, id: string): Promise<string | null> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ org: string | null }>`select resolve_org(${kind}, ${id}::uuid) as org`.execute(trx),
    );
    return rows[0]?.org ?? null;
  }

  /** Organization of a provider installation id (webhooks and login). */
  async installationOrg(provider: string, installationId: string | number): Promise<string | null> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ org: string | null }>`
        select rg_installation_org(${provider}, ${String(installationId)}::bigint) as org`.execute(
        trx,
      ),
    );
    return rows[0]?.org ?? null;
  }

  async roleOf(userId: string, organizationId: string): Promise<MembershipRole | null> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{ role: MembershipRole | null }>`
        select rg_membership_role(${userId}::uuid, ${organizationId}::uuid) as role`.execute(trx),
    );
    return rows[0]?.role ?? null;
  }

  async listForUser(userId: string): Promise<UserMembership[]> {
    const { rows } = await this.dbs.withTx(null, (trx) =>
      sql<{
        organization_id: string;
        slug: string;
        display_name: string;
        role: MembershipRole;
      }>`select * from rg_user_memberships(${userId}::uuid)`.execute(trx),
    );
    return rows.map((r) => ({
      organizationId: r.organization_id,
      slug: r.slug,
      displayName: r.display_name,
      role: r.role,
    }));
  }
}
