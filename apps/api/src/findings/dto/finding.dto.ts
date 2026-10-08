import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';

/** Severity ordering: critical > high > medium > low > info. */
export const SEVERITIES = ['critical', 'high', 'medium', 'low', 'info'] as const;
export type Severity = (typeof SEVERITIES)[number];
export const SEVERITY_RANK: Record<Severity, number> = {
  critical: 4,
  high: 3,
  medium: 2,
  low: 1,
  info: 0,
};

export const REVIEWERS = [
  'correctness',
  'security',
  'test',
  'architecture',
  'performance',
  'maintainability',
] as const;

/** Where a finding stands: published > verified (not published) > suppressed > candidate. */
export const LIFECYCLES = ['published', 'verified', 'suppressed', 'candidate'] as const;

const uuid = z.string().uuid();
const csv = <T extends readonly [string, ...string[]]>(values: T) =>
  z
    .string()
    .max(200)
    .transform((v) => v.split(',').filter(Boolean))
    .pipe(z.array(z.enum(values)));

export const ListFindingsQuerySchema = z.object({
  state: z.enum(['published', 'verified', 'suppressed', 'all']).default('all'),
  /** Comma separated, e.g. `critical,high`. */
  severity: csv(SEVERITIES).optional(),
  /** Comma separated reviewer types. */
  reviewer: csv(REVIEWERS).optional(),
});

/**
 * One typed evidence item (DOM-007). Code excerpts are never stored: a location is a reference
 * (`path`, lines, snapshot) and the UI fetches a redacted excerpt through the graph proxy.
 */
export const EvidenceItemSchema = z.object({
  kind: z.string(),
  claimed_strength: z.string().nullable(),
  origin: z.unknown(),
  verification: z.unknown(),
  /** Model- or tool-authored claim (at most 500 characters); untrusted when rendered. */
  claim: z.string().nullable(),
  location: z.unknown().nullable(),
  symbols: z.array(z.unknown()),
  relation: z.unknown().nullable(),
});

export const SuppressionSchema = z.object({
  reason: z.string(),
  detail: z.string().nullable(),
  stage: z.number().int().nullable(),
  /** For duplicates: the candidate this one was merged into. */
  duplicate_of: z.string().nullable(),
});

export const AnchorSchema = z.object({
  path: z.string(),
  side: z.enum(['head', 'base']),
  start_line: z.number().int(),
  end_line: z.number().int(),
});

export const PublicationSchema = z.object({
  id: uuid,
  placement: z.enum(['inline', 'summary']),
  path: z.string().nullable(),
  start_line: z.number().int().nullable(),
  end_line: z.number().int().nullable(),
  head_sha: z.string(),
  provider_review_id: z.string().nullable(),
  provider_comment_id: z.string().nullable(),
  published_at: z.string(),
});

export const FindingSummarySchema = z.object({
  /** The verified finding id, or the candidate id for a finding that never got verified. */
  id: uuid,
  candidate_id: uuid,
  verified_id: uuid.nullable(),
  review_run_id: uuid,
  lifecycle: z.enum(LIFECYCLES),
  state: z.string(),
  reviewer: z.string(),
  category: z.string(),
  title: z.string(),
  severity: z.enum(SEVERITIES),
  /** Computed by verification (never the model's self-report); null before verification. */
  confidence: z.number().nullable(),
  band: z.string().nullable(),
  anchor: AnchorSchema,
  suppression: SuppressionSchema.nullable(),
  published: z
    .object({
      placement: z.string(),
      published_at: z.string(),
      provider_comment_id: z.string().nullable(),
    })
    .nullable(),
  created_at: z.string(),
});

export const FindingListSchema = z.object({ items: z.array(FindingSummarySchema) });

/** `FindingDetail`: anchor, symbols, explanation, typed evidence, severity, confidence, publication. */
export const FindingDetailSchema = FindingSummarySchema.extend({
  explanation: z.string(),
  symbols: z.array(z.string()),
  evidence: z.array(EvidenceItemSchema),
  confidence_detail: z
    .object({
      value: z.number(),
      /** Components of the computed confidence (VER-009), when recorded. */
      components: z.record(z.string(), z.number()).nullable(),
    })
    .nullable(),
  severity_candidate: z.enum(SEVERITIES),
  verification_version: z.number().int().nullable(),
  publication: PublicationSchema.nullable(),
  repository_id: uuid,
  pull_request_id: uuid,
});

const StageSchema = z.object({
  stage: z.number().int(),
  outcome: z.enum(['pass', 'fail', 'inconclusive']),
  reason: z.string().nullable(),
});

/**
 * `FindingTrace` (PRD sections 85 and 86): change, symbols, context references, reviewer and
 * version, candidate, verification stages with evidence, dedup merges, effective policy and
 * publication. It never carries prompts or model output beyond the typed claims.
 */
export const FindingTraceSchema = z.object({
  finding_id: uuid,
  change: z.object({
    review_run_id: uuid,
    pull_request_id: uuid,
    base_sha: z.string(),
    head_sha: z.string(),
    anchor: AnchorSchema,
  }),
  symbols: z.array(z.string()),
  /** Code locations the reviewer was shown or cited (references only). */
  context_refs: z.array(
    z.object({
      path: z.string(),
      start_line: z.number().int().nullable(),
      end_line: z.number().int().nullable(),
      snapshot_id: z.string().nullable(),
    }),
  ),
  /** The ordered symbol path the evidence claims (from -> via... -> to), when there is one. */
  symbol_path: z.array(z.string()),
  reviewer: z.object({
    type: z.string(),
    version: z.string().nullable(),
    model: z.string().nullable(),
    reviewer_run_id: uuid,
  }),
  candidate: z.object({
    id: uuid,
    state: z.string(),
    severity: z.enum(SEVERITIES),
    fingerprint: z.string(),
    created_at: z.string(),
  }),
  verification: z
    .object({
      verified_id: uuid,
      version: z.number().int(),
      stages: z.array(StageSchema),
      evidence: z.array(EvidenceItemSchema),
      confidence: z.number(),
      severity: z.enum(SEVERITIES),
    })
    .nullable(),
  dedup: z.object({
    /** This candidate was merged into another one. */
    merged_into: z.string().nullable(),
    /** Candidates merged into this one. */
    merged_from: z.array(uuid),
  }),
  policy: z.object({ band: z.string().nullable(), suppression: SuppressionSchema.nullable() }),
  publication: PublicationSchema.nullable(),
  /** True when an older run lacks some stage rows; the available items are still returned. */
  incomplete: z.boolean(),
});

export class ListFindingsQueryDto extends createZodDto(ListFindingsQuerySchema) {}
export class FindingListDto extends createZodDto(FindingListSchema) {}
export class FindingDetailDto extends createZodDto(FindingDetailSchema) {}
export class FindingTraceDto extends createZodDto(FindingTraceSchema) {}

export type ListFindingsQuery = z.infer<typeof ListFindingsQuerySchema>;
export type FindingSummary = z.infer<typeof FindingSummarySchema>;
export type FindingDetail = z.infer<typeof FindingDetailSchema>;
export type FindingTrace = z.infer<typeof FindingTraceSchema>;
export type EvidenceItem = z.infer<typeof EvidenceItemSchema>;
