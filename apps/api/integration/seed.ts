import { randomInt, randomUUID } from 'node:crypto';
import type { Kysely } from 'kysely';
import type { DB } from '../src/db/generated';
import { createKysely, createPool } from '../src/db/kysely.provider';

/** Plain (not role-switched) connection used to seed and clean up; the dev login is a superuser. */
export function adminDb(max = 2): Kysely<DB> {
  return createKysely(createPool({ connectionString: process.env.RG_TEST_DATABASE_URL!, max }));
}

export interface SeededOrg {
  organizationId: string;
  installationId: string;
  providerInstallationId: number;
  repositoryIds: string[];
  slug: string;
}

export const unique = (prefix: string): string => `${prefix}-${randomUUID().slice(0, 8)}`;

export function newProviderInstallationId(): number {
  return randomInt(1_000_000_000, 2_000_000_000);
}

/** One organization with one installation and `repos` repositories. */
export async function seedOrg(db: Kysely<DB>, repos = 1): Promise<SeededOrg> {
  const slug = unique('it');
  const org = await db
    .insertInto('organizations')
    .values({ slug, display_name: slug })
    .returning('id')
    .executeTakeFirstOrThrow();
  const providerInstallationId = newProviderInstallationId();
  const installation = await db
    .insertInto('provider_installations')
    .values({
      organization_id: org.id,
      provider: 'github',
      provider_installation_id: providerInstallationId,
      account_login: slug,
      account_type: 'organization',
    })
    .returning('id')
    .executeTakeFirstOrThrow();
  const repositoryIds: string[] = [];
  for (let i = 0; i < repos; i++) {
    const repo = await db
      .insertInto('repositories')
      .values({
        organization_id: org.id,
        installation_id: installation.id,
        provider: 'github',
        provider_repo_id: String(randomInt(1, 2_000_000_000)),
        full_name: `${slug}/repo-${i}`,
        default_branch: 'main',
        visibility: 'private',
      })
      .returning('id')
      .executeTakeFirstOrThrow();
    repositoryIds.push(repo.id);
  }
  return {
    organizationId: org.id,
    installationId: installation.id,
    providerInstallationId,
    repositoryIds,
    slug,
  };
}

export async function seedUser(db: Kysely<DB>): Promise<string> {
  const login = unique('user');
  const user = await db
    .insertInto('users')
    .values({ provider: 'github', provider_user_id: String(randomInt(1, 2_000_000_000)), login })
    .returning('id')
    .executeTakeFirstOrThrow();
  return user.id;
}

export async function addMember(
  db: Kysely<DB>,
  organizationId: string,
  userId: string,
  role: 'owner' | 'admin' | 'member' | 'viewer',
): Promise<void> {
  await db
    .insertInto('memberships')
    .values({ organization_id: organizationId, user_id: userId, role })
    .execute();
}

/** Deleting the organization cascades to every tenant row. */
export async function cleanup(
  db: Kysely<DB>,
  orgIds: string[],
  userIds: string[] = [],
): Promise<void> {
  if (orgIds.length) await db.deleteFrom('organizations').where('id', 'in', orgIds).execute();
  if (userIds.length) await db.deleteFrom('users').where('id', 'in', userIds).execute();
}
