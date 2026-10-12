import type { Kysely } from 'kysely';
import type { DB } from '../../src/db/generated';
import type { TenantIdParam } from '../../test/security/isolation-matrix';
import {
  addMember,
  seedFinding,
  seedOrg,
  seedPullRequest,
  seedReviewerRun,
  seedReviewJob,
  seedReviewRun,
  seedUser,
  type SeededOrg,
} from '../seed';

/**
 * One tenant of the two-organization isolation fixture (SEC-001): an owner, a repository with
 * settings, a pull request, a review run with a reviewer run, a published finding with feedback,
 * a queued job and an audit row. Both tenants get identical-looking data (same repository and
 * file names), so a leak is detectable only by id.
 */
export interface Tenant {
  org: SeededOrg;
  owner: string;
  ids: Record<TenantIdParam, string>;
  pullRequestId: string;
  reviewRunId: string;
  findingId: string;
  installationId: string;
}

export async function buildTenant(admin: Kysely<DB>): Promise<Tenant> {
  const org = await seedOrg(admin, 1);
  const owner = await seedUser(admin);
  await addMember(admin, org.organizationId, owner, 'owner');
  const repoId = org.repositoryIds[0]!;
  await admin
    .insertInto('repository_settings')
    .values({ repository_id: repoId, organization_id: org.organizationId })
    .execute();
  const pr = await seedPullRequest(admin, org);
  const run = await seedReviewRun(admin, org, pr, { state: 'REVIEWING' });
  const reviewer = await seedReviewerRun(admin, org, run, 'security', 'succeeded');
  const finding = await seedFinding(admin, org, run, reviewer, { published: true });
  await admin
    .insertInto('feedback')
    .values({
      organization_id: org.organizationId,
      repository_id: repoId,
      finding_id: finding.verifiedId!,
      user_id: owner,
      source: 'web',
      verdict: 'useful',
    })
    .execute();
  await seedReviewJob(admin, org, run);
  await admin
    .insertInto('audit_log')
    .values({
      organization_id: org.organizationId,
      actor_type: 'system',
      action: 'repository.enabled',
      target_type: 'repository',
      target_id: repoId,
    })
    .execute();
  return {
    org,
    owner,
    ids: {
      repoId,
      pullRequestId: pr,
      reviewId: run,
      findingId: finding.verifiedId!,
      organizationId: org.organizationId,
    },
    pullRequestId: pr,
    reviewRunId: run,
    findingId: finding.verifiedId!,
    installationId: org.installationId,
  };
}
