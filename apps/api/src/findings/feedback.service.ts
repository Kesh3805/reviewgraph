import {
  ConflictException,
  ForbiddenException,
  HttpStatus,
  Inject,
  Injectable,
  NotFoundException,
} from '@nestjs/common';
import { sql, type Selectable } from 'kysely';
import { AuditService } from '../audit/audit.service';
import { incCounter } from '../common/metrics';
import { ProblemException } from '../common/problem.filter';
import { DbService } from '../db/db.module';
import type { DB } from '../db/generated';
import type { Tx } from '../db/tx';
import type { MembershipRole } from '../tenancy/request-context';
import { satisfies } from '../tenancy/roles.decorator';
import {
  FeedbackRequestSchema,
  SUPPRESSIBLE_VERDICTS,
  VERDICTS,
  type FeedbackRequest,
  type FeedbackResponse,
  type FeedbackResult,
  type FeedbackSummary,
  type Verdict,
} from './dto/feedback.dto';
import { loadFinding, type FindingRow } from './finding-rows';
import { SUPPRESSION_WRITER, type SuppressionWriter } from './suppression.port';

const SUMMARY_DEFAULT_DAYS = 30;

type FeedbackRow = Selectable<DB['feedback']>;

export interface FeedbackActor {
  userId: string;
  role: MembershipRole;
}

/**
 * Finding feedback (API-012, PRD section 70). A user's repeated feedback on a finding updates
 * their row (the latest verdict wins) and every change is audited with the previous verdict.
 */
@Injectable()
export class FeedbackService {
  constructor(
    private readonly dbs: DbService,
    private readonly audit: AuditService,
    @Inject(SUPPRESSION_WRITER) private readonly suppressions: SuppressionWriter,
  ) {}

  /** Validates a raw request body: an invalid verdict or shape answers 422. */
  parseRequest(body: unknown): FeedbackRequest {
    const parsed = FeedbackRequestSchema.safeParse(body);
    if (!parsed.success) {
      throw new ProblemException(HttpStatus.UNPROCESSABLE_ENTITY, 'invalid feedback', {
        errors: parsed.error.issues.map((i) => ({ path: i.path.join('.'), message: i.message })),
      });
    }
    return parsed.data;
  }

  async submit(
    orgId: string,
    actor: FeedbackActor,
    findingId: string,
    request: FeedbackRequest,
  ): Promise<FeedbackResult> {
    if (request.create_suppression) {
      if (!SUPPRESSIBLE_VERDICTS.includes(request.verdict)) {
        throw new ProblemException(
          HttpStatus.UNPROCESSABLE_ENTITY,
          'a suppression can only be created with the intentional or not_relevant verdicts',
        );
      }
      if (!satisfies(actor.role, 'maintainer')) {
        throw new ForbiddenException('creating a suppression requires the maintainer role');
      }
    }
    const result = await this.dbs.withTx(orgId, async (trx) => {
      const finding = await this.loadVerified(trx, findingId);
      const run = await trx
        .selectFrom('review_runs')
        .select('repository_id')
        .where('id', '=', finding.review_run_id)
        .executeTakeFirstOrThrow();
      const previous = await trx
        .selectFrom('feedback')
        .select('verdict')
        .where('finding_id', '=', finding.v_id)
        .where('user_id', '=', actor.userId)
        .where('source', '=', 'web')
        .executeTakeFirst();
      const row = await trx
        .insertInto('feedback')
        .values({
          organization_id: orgId,
          repository_id: run.repository_id,
          finding_id: finding.v_id,
          user_id: actor.userId,
          source: 'web',
          verdict: request.verdict,
          comment: request.comment ?? null,
        })
        .onConflict((oc) =>
          oc.columns(['finding_id', 'user_id', 'source']).doUpdateSet({
            verdict: request.verdict,
            comment: request.comment ?? null,
          }),
        )
        .returningAll()
        .executeTakeFirstOrThrow();
      await this.audit.record(trx, {
        organizationId: orgId,
        repositoryId: run.repository_id,
        actor: { type: 'user', id: actor.userId },
        action: previous ? 'finding.feedback.updated' : 'finding.feedback.created',
        targetType: 'finding',
        targetId: finding.v_id,
        metadata: {
          feedback_id: row.id,
          verdict: request.verdict,
          previous_verdict: previous?.verdict ?? null,
          has_comment: Boolean(request.comment),
        },
      });

      let suppressionId: string | null = null;
      if (request.create_suppression) {
        const { kind, reason } = request.create_suppression;
        const value = suppressionValue(finding, kind);
        suppressionId = await this.suppressions.create(trx, {
          organizationId: orgId,
          repositoryId: run.repository_id,
          kind,
          value,
          reason,
          createdBy: actor.userId,
        });
        await this.audit.record(trx, {
          organizationId: orgId,
          repositoryId: run.repository_id,
          actor: { type: 'user', id: actor.userId },
          action: 'suppression.created',
          targetType: 'suppression',
          targetId: suppressionId,
          metadata: { kind, finding_id: finding.v_id, feedback_id: row.id },
        });
      }
      return { row, suppressionId, reviewer: finding.reviewer };
    });
    incCounter('finding_feedback_total', {
      verdict: request.verdict,
      reviewer: result.reviewer,
    });
    return { feedback: toFeedback(result.row), suppression_id: result.suppressionId };
  }

  async list(orgId: string, findingId: string): Promise<{ items: FeedbackResponse[] }> {
    const rows = await this.dbs.withTx(orgId, async (trx) => {
      const finding = await loadFinding(trx, findingId);
      if (!finding) throw new NotFoundException();
      if (!finding.v_id) return [];
      return trx
        .selectFrom('feedback')
        .selectAll()
        .where('finding_id', '=', finding.v_id)
        .orderBy('updated_at', 'desc')
        .orderBy('id')
        .execute();
    });
    return { items: rows.map(toFeedback) };
  }

  /**
   * Acceptance and false-positive rates of a repository since a date (default 30 days), overall
   * and per reviewer. Rates are over all feedback rows, web and provider alike.
   */
  async summary(orgId: string, repositoryId: string, since?: string): Promise<FeedbackSummary> {
    const from = since ? new Date(since) : new Date(Date.now() - SUMMARY_DEFAULT_DAYS * 86_400_000);
    const rows = await this.dbs.withTx(orgId, async (trx) => {
      const repo = await trx
        .selectFrom('repositories')
        .select('id')
        .where('id', '=', repositoryId)
        .executeTakeFirst();
      if (!repo) throw new NotFoundException();
      return trx
        .selectFrom('feedback as f')
        .innerJoin('verified_findings as v', 'v.id', 'f.finding_id')
        .innerJoin('candidate_findings as c', 'c.id', 'v.candidate_finding_id')
        .select(['c.reviewer', 'f.verdict', sql<string>`count(*)`.as('n')])
        .where('f.repository_id', '=', repositoryId)
        .where('f.updated_at', '>=', from)
        .groupBy(['c.reviewer', 'f.verdict'])
        .execute();
    });
    const overall = new Map<string, number>();
    const perReviewer = new Map<string, Map<string, number>>();
    for (const r of rows) {
      const n = Number(r.n);
      overall.set(r.verdict, (overall.get(r.verdict) ?? 0) + n);
      const m = perReviewer.get(r.reviewer) ?? new Map<string, number>();
      m.set(r.verdict, (m.get(r.verdict) ?? 0) + n);
      perReviewer.set(r.reviewer, m);
    }
    return {
      repository_id: repositoryId,
      since: from.toISOString(),
      ...rates(overall),
      by_reviewer: Object.fromEntries(
        [...perReviewer].sort(([a], [b]) => a.localeCompare(b)).map(([k, m]) => [k, rates(m)]),
      ),
    };
  }

  /**
   * A weak signal from the provider (a reaction or reply on our comment, GH-011): recorded with
   * `source='provider'` and no user, in the caller's transaction.
   */
  async recordProviderSignal(
    trx: Tx,
    signal: { organizationId: string; findingId: string; verdict: Verdict },
  ): Promise<void> {
    const finding = await this.loadVerified(trx, signal.findingId);
    const run = await trx
      .selectFrom('review_runs')
      .select('repository_id')
      .where('id', '=', finding.review_run_id)
      .executeTakeFirstOrThrow();
    await trx
      .insertInto('feedback')
      .values({
        organization_id: signal.organizationId,
        repository_id: run.repository_id,
        finding_id: finding.v_id,
        user_id: null,
        source: 'provider',
        verdict: signal.verdict,
      })
      .execute();
    incCounter('finding_feedback_total', { verdict: signal.verdict, reviewer: finding.reviewer });
  }

  private async loadVerified(trx: Tx, findingId: string): Promise<FindingRow & { v_id: string }> {
    const finding = await loadFinding(trx, findingId);
    if (!finding) throw new NotFoundException();
    if (!finding.v_id) {
      throw new ConflictException('feedback needs a verified finding');
    }
    return finding as FindingRow & { v_id: string };
  }
}

function suppressionValue(finding: FindingRow, kind: 'fingerprint' | 'symbol' | 'path'): string {
  if (kind === 'fingerprint') return finding.fingerprint;
  if (kind === 'path') return finding.changed_path;
  const symbol = finding.affected_symbols[0];
  if (!symbol) {
    throw new ProblemException(
      HttpStatus.UNPROCESSABLE_ENTITY,
      'the finding names no symbol to suppress',
    );
  }
  return symbol;
}

function rates(counts: Map<string, number>) {
  const total = [...counts.values()].reduce((a, b) => a + b, 0);
  const of = (verdict: Verdict) => counts.get(verdict) ?? 0;
  return {
    total,
    by_verdict: Object.fromEntries(VERDICTS.map((v) => [v, of(v)])),
    acceptance_rate: total === 0 ? null : of('useful') / total,
    false_positive_rate: total === 0 ? null : of('false_positive') / total,
  };
}

function toFeedback(row: FeedbackRow): FeedbackResponse {
  return {
    id: row.id,
    finding_id: row.finding_id,
    repository_id: row.repository_id,
    user_id: row.user_id,
    source: row.source as FeedbackResponse['source'],
    verdict: row.verdict as Verdict,
    comment: row.comment,
    created_at: row.created_at.toISOString(),
    updated_at: row.updated_at.toISOString(),
  };
}
