import {
  ConflictException,
  HttpStatus,
  Inject,
  Injectable,
  NotFoundException,
  ServiceUnavailableException,
} from '@nestjs/common';
import type { Redis } from 'ioredis';
import { sql, type Selectable } from 'kysely';
import { AuditService } from '../audit/audit.service';
import { ProblemException } from '../common/problem.filter';
import { incCounter } from '../common/metrics';
import { REDIS } from '../common/redis.module';
import { DbService } from '../db/db.module';
import type { DB } from '../db/generated';
import type { Tx } from '../db/tx';
import { JOB_QUEUE, type JobQueue } from '../jobs/job-queue.port';
import {
  PROVIDER_RESOLVER,
  ProviderError,
  type ProviderKind,
  type ProviderResolver,
  type RepoRef,
} from '../providers/ports';
import type {
  RepositoryResponse,
  RepositorySettingsResponse,
  RepositoryStatusResponse,
  UpdateRepositorySettingsInput,
} from './dto/repository.dto';
import { PROFILE_READER, type ProfileReader } from './profile.port';

export const REBUILD_LIMIT_SECONDS = 3600;
export const rebuildLimitKey = (repositoryId: string): string => `rg:repo-rebuild:${repositoryId}`;

const DEFAULT_SETTINGS: RepositorySettingsResponse = {
  enabled: true,
  target_branches: [],
  skip_drafts: true,
  skip_bots: true,
  reviewer_overrides: {},
};

type RepositoryRow = Selectable<DB['repositories']> & {
  s_enabled: boolean | null;
  s_target_branches: string[] | null;
  s_skip_drafts: boolean | null;
  s_skip_bots: boolean | null;
  s_reviewer_overrides: unknown;
};

export interface ListQuery {
  limit: number;
  cursor?: string;
}

/** Repository onboarding, status and settings (API-008). Every method runs tenant scoped. */
@Injectable()
export class RepositoriesService {
  constructor(
    private readonly dbs: DbService,
    private readonly audit: AuditService,
    @Inject(JOB_QUEUE) private readonly queue: JobQueue,
    @Inject(PROVIDER_RESOLVER) private readonly providers: ProviderResolver,
    @Inject(PROFILE_READER) private readonly profiles: ProfileReader,
    @Inject(REDIS) private readonly redis: Redis,
  ) {}

  async list(
    orgId: string,
    query: ListQuery,
  ): Promise<{ items: RepositoryResponse[]; next_cursor: string | null }> {
    const after = decodeCursor(query.cursor);
    const rows = await this.dbs.withTx(orgId, async (trx) => {
      let q = this.selectRepositories(trx).orderBy('r.full_name').orderBy('r.id');
      if (after) q = q.where(sql<boolean>`(r.full_name, r.id) > (${after[0]}, ${after[1]}::uuid)`);
      return q.limit(query.limit + 1).execute();
    });
    const page = rows.slice(0, query.limit);
    const last = page.at(-1);
    return {
      items: page.map(toRepository),
      next_cursor: rows.length > query.limit && last ? encodeCursor(last.full_name, last.id) : null,
    };
  }

  async get(orgId: string, repositoryId: string): Promise<RepositoryResponse> {
    const row = await this.dbs.withTx(orgId, (trx) => this.loadRepository(trx, repositoryId));
    return toRepository(row);
  }

  /**
   * Enables a repository of an installation for review. The repository must already be known
   * (synced from the installation events); enabling is idempotent.
   */
  async enable(
    orgId: string,
    actor: { userId: string },
    input: { installationId: string; fullName: string },
  ): Promise<RepositoryResponse> {
    const row = await this.dbs.withTx(orgId, async (trx) => {
      const installation = await trx
        .selectFrom('provider_installations')
        .select(['id', 'state'])
        .where('id', '=', input.installationId)
        .executeTakeFirst();
      if (!installation) throw new NotFoundException();
      if (installation.state !== 'active') {
        throw new ConflictException(`the installation is ${installation.state}`);
      }
      const repo = await trx
        .selectFrom('repositories')
        .select(['id', 'enabled', 'access_state'])
        .where('installation_id', '=', installation.id)
        .where(sql<boolean>`lower(full_name) = ${input.fullName.toLowerCase()}`)
        .executeTakeFirst();
      if (!repo) {
        throw new NotFoundException(
          'the installation has no such repository (it appears after the installation event or sync)',
        );
      }
      if (!repo.enabled) {
        throw new ConflictException(`repository access is ${repo.access_state}`);
      }
      const before = await this.loadSettings(trx, repo.id);
      await this.saveSettings(trx, orgId, repo.id, { ...before, enabled: true });
      if (!before.enabled || !before.exists) {
        await this.audit.record(trx, {
          organizationId: orgId,
          repositoryId: repo.id,
          actor: { type: 'user', id: actor.userId },
          action: 'repository.enabled',
          targetType: 'repository',
          targetId: repo.id,
          metadata: { installation_id: installation.id },
        });
      }
      return this.loadRepository(trx, repo.id);
    });
    return toRepository(row);
  }

  async status(orgId: string, repositoryId: string): Promise<RepositoryStatusResponse> {
    return this.dbs.withTx(orgId, async (trx) => {
      const repo = await this.loadRepository(trx, repositoryId);
      const facts = repo.latest_init_facts_id
        ? await trx
            .selectFrom('repository_init_facts')
            .select([
              'commit_sha',
              'fingerprint',
              'facts_hash',
              'facts_schema_version',
              'tool_version',
              'primary_language',
              'is_monorepo',
              'frameworks',
              'warnings_count',
              'detected_at',
            ])
            .where('id', '=', repo.latest_init_facts_id)
            .executeTakeFirst()
        : undefined;
      const active = await this.queue.findActive(trx, 'repository-index', repositoryId);
      const profile = await this.profiles.read(trx, repositoryId);
      return {
        repository_id: repositoryId,
        index_state: active
          ? active.state === 'running'
            ? 'indexing'
            : 'queued'
          : facts
            ? 'ready'
            : 'not_initialized',
        init: facts
          ? {
              commit_sha: facts.commit_sha,
              fingerprint: facts.fingerprint,
              facts_hash: facts.facts_hash,
              facts_schema_version: facts.facts_schema_version,
              tool_version: facts.tool_version,
              primary_language: facts.primary_language,
              is_monorepo: facts.is_monorepo,
              frameworks: facts.frameworks,
              warnings_count: facts.warnings_count,
              detected_at: facts.detected_at.toISOString(),
            }
          : null,
        // Graph snapshots, repository config validation and profiles are later tasks.
        last_snapshot: null,
        active_job: active ? { id: active.jobId, queue: active.queue, state: active.state } : null,
        config: { hash: null, validation_errors: [] },
        profile_computed_at: profile ? profile.computedAt.toISOString() : null,
      };
    });
  }

  /** The profile JSON; 404 before the first index. */
  async profile(orgId: string, repositoryId: string): Promise<unknown> {
    const profile = await this.dbs.withTx(orgId, async (trx) => {
      await this.loadRepository(trx, repositoryId);
      return this.profiles.read(trx, repositoryId);
    });
    if (!profile) throw new NotFoundException('the repository has no profile yet');
    return profile.profile;
  }

  /** Enqueues `repository-index` for the default-branch head (idempotent per head). */
  async initialize(
    orgId: string,
    repositoryId: string,
  ): Promise<{ job_id: string; idempotency_key: string; head_sha: string }> {
    const repo = await this.dbs.withTx(orgId, (trx) => this.loadForIndexing(trx, repositoryId));
    const headSha = await this.defaultBranchHead(repo);
    const idempotencyKey = `repo-index:${repositoryId}:${headSha}`;
    const result = await this.dbs.withTx(orgId, (trx) =>
      this.queue.enqueue(trx, {
        queue: 'repository-index',
        idempotencyKey,
        payload: {
          repository_id: repositoryId,
          organization_id: orgId,
          head_sha: headSha,
          force: false,
        },
      }),
    );
    incCounter('repository_index_requests_total', { kind: 'initialize' });
    if (!result.created) {
      throw new ProblemException(HttpStatus.CONFLICT, 'an index job already exists for this head', {
        job_id: result.jobId,
      });
    }
    return { job_id: result.jobId, idempotency_key: idempotencyKey, head_sha: headSha };
  }

  /** Enqueues a forced full index; at most one rebuild per repository per hour. */
  async rebuild(
    orgId: string,
    repositoryId: string,
  ): Promise<{ job_id: string; idempotency_key: string; head_sha: string }> {
    const repo = await this.dbs.withTx(orgId, (trx) => this.loadForIndexing(trx, repositoryId));
    const headSha = await this.defaultBranchHead(repo);

    const limitKey = rebuildLimitKey(repositoryId);
    const claimed = await this.claimRebuildSlot(limitKey);
    if (!claimed.ok) {
      incCounter('repository_index_requests_total', { kind: 'rebuild_limited' });
      throw new ProblemException(
        HttpStatus.TOO_MANY_REQUESTS,
        'a rebuild was already requested for this repository within the last hour',
        { retry_after_seconds: claimed.retryAfter },
        { 'Retry-After': String(claimed.retryAfter) },
      );
    }

    const idempotencyKey = `repo-rebuild:${repositoryId}:${headSha}:${hourBucket(new Date())}`;
    try {
      const result = await this.dbs.withTx(orgId, (trx) =>
        this.queue.enqueue(trx, {
          queue: 'repository-index',
          idempotencyKey,
          payload: {
            repository_id: repositoryId,
            organization_id: orgId,
            head_sha: headSha,
            force: true,
          },
        }),
      );
      if (!result.created) {
        await this.releaseRebuildSlot(limitKey);
        throw new ProblemException(
          HttpStatus.CONFLICT,
          'a rebuild job already exists for this head',
          {
            job_id: result.jobId,
          },
        );
      }
      incCounter('repository_index_requests_total', { kind: 'rebuild' });
      return { job_id: result.jobId, idempotency_key: idempotencyKey, head_sha: headSha };
    } catch (err) {
      // Nothing was queued: do not burn the hourly slot.
      if (!(err instanceof ProblemException)) await this.releaseRebuildSlot(limitKey);
      throw err;
    }
  }

  async updateSettings(
    orgId: string,
    actor: { userId: string },
    repositoryId: string,
    patch: UpdateRepositorySettingsInput,
  ): Promise<RepositorySettingsResponse> {
    return this.dbs.withTx(orgId, async (trx) => {
      await this.loadRepository(trx, repositoryId);
      const before = await this.loadSettings(trx, repositoryId);
      const current: RepositorySettingsResponse = {
        enabled: before.enabled,
        target_branches: before.target_branches,
        skip_drafts: before.skip_drafts,
        skip_bots: before.skip_bots,
        reviewer_overrides: before.reviewer_overrides,
      };
      const next: RepositorySettingsResponse = {
        ...current,
        ...patch,
        reviewer_overrides: { ...(patch.reviewer_overrides ?? current.reviewer_overrides) },
      };
      const changed = diffSettings(current, next);
      await this.saveSettings(trx, orgId, repositoryId, next);
      if (Object.keys(changed).length > 0) {
        // Same transaction as the change: no settings change without its audit row.
        await this.audit.record(trx, {
          organizationId: orgId,
          repositoryId,
          actor: { type: 'user', id: actor.userId },
          action: 'repository.settings.updated',
          targetType: 'repository',
          targetId: repositoryId,
          metadata: { changed },
        });
      }
      return next;
    });
  }

  // --- helpers ---

  private selectRepositories(trx: Tx) {
    return trx
      .selectFrom('repositories as r')
      .leftJoin('repository_settings as s', 's.repository_id', 'r.id')
      .selectAll('r')
      .select([
        's.enabled as s_enabled',
        's.target_branches as s_target_branches',
        's.skip_drafts as s_skip_drafts',
        's.skip_bots as s_skip_bots',
        's.reviewer_overrides as s_reviewer_overrides',
      ]);
  }

  private async loadRepository(trx: Tx, repositoryId: string): Promise<RepositoryRow> {
    const row = await this.selectRepositories(trx)
      .where('r.id', '=', repositoryId)
      .executeTakeFirst();
    if (!row) throw new NotFoundException();
    return row as RepositoryRow;
  }

  private async loadSettings(
    trx: Tx,
    repositoryId: string,
  ): Promise<RepositorySettingsResponse & { exists: boolean }> {
    const row = await trx
      .selectFrom('repository_settings')
      .selectAll()
      .where('repository_id', '=', repositoryId)
      .executeTakeFirst();
    if (!row) return { ...DEFAULT_SETTINGS, exists: false };
    return {
      enabled: row.enabled,
      target_branches: row.target_branches,
      skip_drafts: row.skip_drafts,
      skip_bots: row.skip_bots,
      reviewer_overrides:
        row.reviewer_overrides as RepositorySettingsResponse['reviewer_overrides'],
      exists: true,
    };
  }

  private async saveSettings(
    trx: Tx,
    orgId: string,
    repositoryId: string,
    settings: RepositorySettingsResponse & { exists?: boolean },
  ): Promise<void> {
    const values = {
      enabled: settings.enabled,
      target_branches: settings.target_branches,
      skip_drafts: settings.skip_drafts,
      skip_bots: settings.skip_bots,
      reviewer_overrides: JSON.stringify(settings.reviewer_overrides),
    };
    await trx
      .insertInto('repository_settings')
      .values({ repository_id: repositoryId, organization_id: orgId, ...values })
      .onConflict((oc) => oc.column('repository_id').doUpdateSet(values))
      .execute();
  }

  /** Repository plus the provider coordinates needed to ask the provider about it. */
  private async loadForIndexing(
    trx: Tx,
    repositoryId: string,
  ): Promise<{ ref: RepoRef; defaultBranch: string }> {
    const row = await trx
      .selectFrom('repositories as r')
      .innerJoin('provider_installations as i', (join) =>
        join
          .onRef('i.id', '=', 'r.installation_id')
          .onRef('i.organization_id', '=', 'r.organization_id'),
      )
      .select([
        'r.full_name',
        'r.default_branch',
        'r.provider',
        'r.enabled',
        'r.access_state',
        'i.provider_installation_id',
        'i.state',
      ])
      .where('r.id', '=', repositoryId)
      .executeTakeFirst();
    if (!row) throw new NotFoundException();
    if (!row.enabled) throw new ConflictException(`repository access is ${row.access_state}`);
    if (row.state !== 'active') throw new ConflictException(`the installation is ${row.state}`);
    const [owner, name] = row.full_name.split('/');
    return {
      ref: {
        provider: row.provider as ProviderKind,
        installationId: String(row.provider_installation_id),
        owner: owner ?? '',
        name: name ?? '',
      },
      defaultBranch: row.default_branch,
    };
  }

  /** The head commit of the default branch, as the provider reports it. */
  private async defaultBranchHead(repo: { ref: RepoRef; defaultBranch: string }): Promise<string> {
    try {
      const commit = await this.providers
        .repository(repo.ref.provider)
        .getCommit(repo.ref, repo.defaultBranch);
      return commit.sha;
    } catch (err) {
      if (err instanceof ProviderError && err.kind === 'not_found') {
        throw new ConflictException('the default branch was not found at the provider');
      }
      throw new ServiceUnavailableException('the provider could not be reached');
    }
  }

  private async claimRebuildSlot(
    key: string,
  ): Promise<{ ok: true } | { ok: false; retryAfter: number }> {
    try {
      const set = await this.redis.set(key, '1', 'EX', REBUILD_LIMIT_SECONDS, 'NX');
      if (set === 'OK') return { ok: true };
      const ttl = await this.redis.ttl(key);
      return { ok: false, retryAfter: ttl > 0 ? ttl : REBUILD_LIMIT_SECONDS };
    } catch {
      // Redis down: the per-hour idempotency key still prevents duplicate jobs.
      return { ok: true };
    }
  }

  private async releaseRebuildSlot(key: string): Promise<void> {
    await this.redis.del(key).catch(() => undefined);
  }
}

function hourBucket(date: Date): string {
  return date.toISOString().slice(0, 13).replace(/[-T]/g, '');
}

function diffSettings(
  before: RepositorySettingsResponse,
  after: RepositorySettingsResponse,
): Record<string, { from: unknown; to: unknown }> {
  const changed: Record<string, { from: unknown; to: unknown }> = {};
  for (const key of Object.keys(after) as (keyof RepositorySettingsResponse)[]) {
    if (JSON.stringify(before[key]) !== JSON.stringify(after[key])) {
      changed[key] = { from: before[key], to: after[key] };
    }
  }
  return changed;
}

function toRepository(row: RepositoryRow): RepositoryResponse {
  return {
    id: row.id,
    organization_id: row.organization_id,
    provider: row.provider as 'github',
    provider_repo_id: row.provider_repo_id,
    full_name: row.full_name,
    default_branch: row.default_branch,
    visibility: row.visibility as RepositoryResponse['visibility'],
    archived: row.archived,
    enabled: row.enabled,
    access_state: row.access_state as RepositoryResponse['access_state'],
    primary_language: row.primary_language,
    initialized_at: row.initialized_at ? row.initialized_at.toISOString() : null,
    settings: {
      enabled: row.s_enabled ?? DEFAULT_SETTINGS.enabled,
      target_branches: row.s_target_branches ?? DEFAULT_SETTINGS.target_branches,
      skip_drafts: row.s_skip_drafts ?? DEFAULT_SETTINGS.skip_drafts,
      skip_bots: row.s_skip_bots ?? DEFAULT_SETTINGS.skip_bots,
      reviewer_overrides: (row.s_reviewer_overrides ??
        DEFAULT_SETTINGS.reviewer_overrides) as RepositorySettingsResponse['reviewer_overrides'],
    },
    created_at: row.created_at.toISOString(),
    updated_at: row.updated_at.toISOString(),
  };
}

function encodeCursor(fullName: string, id: string): string {
  return Buffer.from(JSON.stringify([fullName, id])).toString('base64url');
}

function decodeCursor(cursor: string | undefined): [string, string] | null {
  if (!cursor) return null;
  try {
    const parsed: unknown = JSON.parse(Buffer.from(cursor, 'base64url').toString('utf8'));
    if (
      Array.isArray(parsed) &&
      typeof parsed[0] === 'string' &&
      typeof parsed[1] === 'string' &&
      /^[0-9a-f-]{36}$/i.test(parsed[1])
    ) {
      return [parsed[0], parsed[1]];
    }
  } catch {
    // fall through
  }
  throw new ProblemException(HttpStatus.BAD_REQUEST, 'invalid cursor');
}
