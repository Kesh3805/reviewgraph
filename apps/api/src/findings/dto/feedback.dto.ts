import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';

export const VERDICTS = [
  'useful',
  'false_positive',
  'already_handled',
  'not_relevant',
  'intentional',
] as const;
export type Verdict = (typeof VERDICTS)[number];

/** A suppression may only come with these verdicts (the finding is real but not wanted). */
export const SUPPRESSIBLE_VERDICTS: readonly Verdict[] = ['intentional', 'not_relevant'];

export const SUPPRESSION_KINDS = ['fingerprint', 'symbol', 'path'] as const;
export type SuppressionKind = (typeof SUPPRESSION_KINDS)[number];

export const FeedbackRequestSchema = z
  .object({
    verdict: z.enum(VERDICTS),
    /** Plain text; at most 2,000 characters. */
    comment: z.string().max(2000).optional(),
    create_suppression: z
      .object({
        kind: z.enum(SUPPRESSION_KINDS),
        reason: z.string().trim().min(1).max(1000),
      })
      .strict()
      .optional(),
  })
  .strict();

export const FeedbackSchema = z.object({
  id: z.string().uuid(),
  finding_id: z.string().uuid(),
  repository_id: z.string().uuid(),
  user_id: z.string().uuid().nullable(),
  source: z.enum(['web', 'provider']),
  verdict: z.enum(VERDICTS),
  comment: z.string().nullable(),
  created_at: z.string(),
  updated_at: z.string(),
});

export const FeedbackResultSchema = z.object({
  feedback: FeedbackSchema,
  /** The suppression created with the feedback, if one was asked for. */
  suppression_id: z.string().nullable(),
});

export const FeedbackListSchema = z.object({ items: z.array(FeedbackSchema) });

export const FeedbackSummaryQuerySchema = z.object({
  /** ISO timestamp; defaults to 30 days ago. */
  since: z.iso.datetime({ offset: true }).optional(),
});

const rates = {
  total: z.number().int(),
  by_verdict: z.record(z.string(), z.number().int()),
  /** `useful / total` (PRD section 116); null without feedback. */
  acceptance_rate: z.number().nullable(),
  /** `false_positive / total` (PRD section 117); null without feedback. */
  false_positive_rate: z.number().nullable(),
};

export const FeedbackSummarySchema = z.object({
  repository_id: z.string().uuid(),
  since: z.string(),
  ...rates,
  by_reviewer: z.record(z.string(), z.object(rates)),
});

export class FeedbackRequestDto extends createZodDto(FeedbackRequestSchema) {}
export class FeedbackResultDto extends createZodDto(FeedbackResultSchema) {}
export class FeedbackListDto extends createZodDto(FeedbackListSchema) {}
export class FeedbackSummaryQueryDto extends createZodDto(FeedbackSummaryQuerySchema) {}
export class FeedbackSummaryDto extends createZodDto(FeedbackSummarySchema) {}

export type FeedbackRequest = z.infer<typeof FeedbackRequestSchema>;
export type FeedbackResponse = z.infer<typeof FeedbackSchema>;
export type FeedbackResult = z.infer<typeof FeedbackResultSchema>;
export type FeedbackSummary = z.infer<typeof FeedbackSummarySchema>;
