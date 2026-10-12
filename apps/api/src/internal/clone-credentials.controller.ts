import {
  Controller,
  ForbiddenException,
  Header,
  HttpCode,
  HttpStatus,
  Inject,
  Logger,
  NotFoundException,
  Param,
  ParseUUIDPipe,
  Post,
  ServiceUnavailableException,
} from '@nestjs/common';
import { ApiExcludeController } from '@nestjs/swagger';
import { SpanStatusCode, trace } from '@opentelemetry/api';
import { sql } from 'kysely';
import { AuditService } from '../audit/audit.service';
import { incCounter } from '../common/metrics';
import { ProblemException } from '../common/problem.filter';
import { APP_CONFIG, type AppConfig } from '../config/config.module';
import { DbService } from '../db/db.module';
import {
  PROVIDER_RESOLVER,
  ProviderError,
  type ProviderKind,
  type ProviderResolver,
} from '../providers/ports';
import { TRACER_NAME } from '../telemetry/tracer.service';
import { CLONE_JOB_GATE, checkCloneJob, type CloneJobGate } from './clone-job-gate';
import { ServiceAuth, ServiceCaller } from './service-auth.guard';
import type { ServiceTokenClaims } from './service-token';

export const CLONE_CREDENTIAL_USERNAME = 'x-access-token';
/**
 * Requested lifetime. GitHub installation tokens always live one hour and cannot be shortened;
 * the worker uses the token for one checkout and drops it (SEC-005).
 */
const CREDENTIAL_TTL_SECONDS = 600;

/** The permission set and repository a broker credential is limited to (SEC-005 contract). */
export interface CloneCredentialScope {
  repository_ids: number[];
  permissions: { contents: 'read' };
}

export interface CloneCredentialsResponse {
  username: typeof CLONE_CREDENTIAL_USERNAME;
  token: string;
  expires_at: string;
  scope: CloneCredentialScope;
  /** Credential-free remote URL; the worker supplies the token through a credential callback. */
  clone_url: string;
}

/**
 * The git host of an API base URL: `https://api.github.com` -> `https://github.com`, a GitHub
 * Enterprise `https://ghe.example/api/v3` -> `https://ghe.example`, anything else -> its origin.
 */
export function gitHostOf(apiUrl: string): string {
  const url = new URL(apiUrl);
  if (url.hostname === 'api.github.com') return 'https://github.com';
  return url.origin;
}

interface RepositoryRow {
  provider: string;
  provider_repo_id: string;
  full_name: string;
  enabled: boolean;
  access_state: string;
  provider_installation_id: string;
  installation_state: string;
}

/**
 * Clone credential broker (GH-006): `POST /internal/repositories/:id/clone-credentials`.
 * Workers get a short-lived, read-only token for exactly one repository; the App private key
 * never leaves the API. The caller must hold a service token with `scope=clone-credentials` and
 * `repo=:id` (API-005). The token is never logged, and the response is `no-store`.
 */
// Service-to-service only: not part of the public OpenAPI document.
@ApiExcludeController()
@Controller('internal/repositories')
export class CloneCredentialsController {
  private readonly logger = new Logger(CloneCredentialsController.name);

  constructor(
    @Inject(APP_CONFIG) private readonly config: AppConfig,
    private readonly dbs: DbService,
    @Inject(PROVIDER_RESOLVER) private readonly providers: ProviderResolver,
    @Inject(CLONE_JOB_GATE) private readonly jobs: CloneJobGate,
    private readonly audit: AuditService,
  ) {}

  @Post(':id/clone-credentials')
  @HttpCode(HttpStatus.OK)
  @Header('Cache-Control', 'no-store')
  @ServiceAuth({ scopes: ['clone-credentials'], repoParam: 'id' })
  issue(
    @Param('id', new ParseUUIDPipe()) id: string,
    @ServiceCaller() caller: ServiceTokenClaims | undefined,
  ): Promise<CloneCredentialsResponse> {
    return trace
      .getTracer(TRACER_NAME)
      .startActiveSpan(
        'clone_credentials_issue',
        { attributes: { repository_id: id } },
        async (span) => {
          try {
            const response = await this.issueFor(id, caller);
            incCounter('clone_credentials_issued_total');
            return response;
          } catch (err) {
            span.setStatus({ code: SpanStatusCode.ERROR });
            throw err;
          } finally {
            span.end();
          }
        },
      );
  }

  private async issueFor(
    id: string,
    caller: ServiceTokenClaims | undefined,
  ): Promise<CloneCredentialsResponse> {
    const org = await this.dbs.withTx(null, async (trx) => {
      const { rows } = await sql<{ org: string | null }>`
        select resolve_org('repository', ${id}::uuid) as org`.execute(trx);
      return rows[0]?.org ?? null;
    });
    if (!org) throw new NotFoundException();
    // A token bound to another organization never gets this repository's credential.
    if (caller?.org && caller.org !== org) throw new ForbiddenException();

    const repo = await this.dbs.withTx(org, (trx) =>
      trx
        .selectFrom('repositories as r')
        .innerJoin('provider_installations as i', 'i.id', 'r.installation_id')
        .select([
          'r.provider',
          'r.provider_repo_id',
          'r.full_name',
          'r.enabled',
          'r.access_state',
          'i.provider_installation_id',
          'i.state as installation_state',
        ])
        .where('r.id', '=', id)
        .executeTakeFirst(),
    );
    if (!repo) throw new NotFoundException();
    const row = repo as RepositoryRow;
    // SEC-005: a job-bound token only gets credentials while its job runs on this repository.
    const jobCheck = await checkCloneJob(this.jobs, caller?.job, id);
    if (jobCheck === 'not_found') throw new NotFoundException();
    if (jobCheck === 'not_running') throw new ForbiddenException();
    if (row.installation_state !== 'active') {
      throw new ProblemException(HttpStatus.CONFLICT, 'the installation is not active', {
        code:
          row.installation_state === 'suspended'
            ? 'installation_suspended'
            : 'installation_inactive',
      });
    }
    if (!row.enabled || row.access_state !== 'active') {
      throw new ProblemException(HttpStatus.CONFLICT, 'the repository is not accessible', {
        code: 'repository_access_lost',
      });
    }

    const [owner, name] = splitFullName(row.full_name);
    const ref = {
      provider: row.provider as ProviderKind,
      installationId: String(row.provider_installation_id),
      owner,
      name,
    };
    let credential;
    try {
      credential = await this.providers
        .repository(ref.provider)
        .issueCloneCredential(ref, CREDENTIAL_TTL_SECONDS, {
          providerRepoId: row.provider_repo_id,
        });
    } catch (err) {
      if (err instanceof ProviderError && !err.retryable) {
        this.logger.warn(`clone credential refused repository=${id} kind=${err.kind}`);
        throw new ProblemException(HttpStatus.CONFLICT, 'the provider refused the credential', {
          code: err.kind === 'not_found' ? 'repository_access_lost' : 'installation_suspended',
        });
      }
      this.logger.warn(`clone credential unavailable repository=${id}`);
      throw new ServiceUnavailableException();
    }
    const scope: CloneCredentialScope = {
      repository_ids: [Number(row.provider_repo_id)],
      permissions: { contents: 'read' },
    };
    const expiresAt = credential.expiresAt.toISOString();
    // Issuance is audited (SEC-005/SEC-008); without the audit row no credential leaves.
    await this.dbs.withTx(org, (trx) =>
      this.audit.record(trx, {
        organizationId: org,
        repositoryId: id,
        actor: { type: 'service', id: caller?.sub },
        action: 'clone_credentials.issued',
        targetType: 'repository',
        targetId: id,
        metadata: {
          expires_at: expiresAt,
          permissions: scope.permissions,
          ...(caller?.job ? { job_id: caller.job } : {}),
        },
      }),
    );
    this.logger.log(`clone credential issued repository=${id}`);
    return {
      username: CLONE_CREDENTIAL_USERNAME,
      token: credential.token.reveal(),
      expires_at: expiresAt,
      scope,
      clone_url: `${gitHostOf(this.config.GITHUB_API_URL)}/${row.full_name}.git`,
    };
  }
}

function splitFullName(fullName: string): [string, string] {
  const at = fullName.indexOf('/');
  return [fullName.slice(0, at), fullName.slice(at + 1)];
}
