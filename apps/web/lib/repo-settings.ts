import { z } from 'zod';
import type { RepositorySettings, RepositorySettingsPatch } from './api/pending';

/**
 * Mirrors `UpdateRepositorySettingsSchema` in apps/api/src/repositories/dto/repository.dto.ts
 * (the contracts schema for `PATCH /repositories/:id/settings`), adapted to form inputs:
 * target branches are edited as comma or newline separated text.
 */
export const REVIEWER_TYPES = [
  'correctness',
  'security',
  'test',
  'architecture',
  'performance',
  'maintainability',
] as const;
export type ReviewerType = (typeof REVIEWER_TYPES)[number];

export const MAX_TARGET_BRANCHES = 50;

export type OverrideChoice = 'default' | 'on' | 'off';

export function parseBranches(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((b) => b.trim())
    .filter((b) => b.length > 0);
}

export const repoSettingsFormSchema = z.object({
  enabled: z.boolean(),
  skip_drafts: z.boolean(),
  skip_bots: z.boolean(),
  target_branches: z.string().superRefine((text, ctx) => {
    const branches = parseBranches(text);
    if (branches.length > MAX_TARGET_BRANCHES) {
      ctx.addIssue({ code: 'custom', message: `At most ${MAX_TARGET_BRANCHES} branch patterns.` });
    }
    for (const b of branches) {
      if (/\s/.test(b)) {
        ctx.addIssue({ code: 'custom', message: `"${b}" must not contain whitespace.` });
      } else if (b.length > 255) {
        ctx.addIssue({ code: 'custom', message: 'Branch patterns are at most 255 characters.' });
      }
    }
  }),
  reviewer_overrides: z.object(
    Object.fromEntries(REVIEWER_TYPES.map((r) => [r, z.enum(['default', 'on', 'off'])])) as Record<
      ReviewerType,
      z.ZodEnum<{ default: 'default'; on: 'on'; off: 'off' }>
    >,
  ),
});

export type RepoSettingsForm = z.infer<typeof repoSettingsFormSchema>;

export function toFormValues(settings: RepositorySettings): RepoSettingsForm {
  const overrides = settings.reviewer_overrides as Partial<Record<ReviewerType, boolean>>;
  return {
    enabled: settings.enabled,
    skip_drafts: settings.skip_drafts,
    skip_bots: settings.skip_bots,
    target_branches: settings.target_branches.join(', '),
    reviewer_overrides: Object.fromEntries(
      REVIEWER_TYPES.map((r) => [
        r,
        overrides[r] === undefined ? 'default' : overrides[r] ? 'on' : 'off',
      ]),
    ) as Record<ReviewerType, OverrideChoice>,
  };
}

export function toPatch(values: RepoSettingsForm): RepositorySettingsPatch {
  const reviewer_overrides: Partial<Record<ReviewerType, boolean>> = {};
  for (const r of REVIEWER_TYPES) {
    const choice = values.reviewer_overrides[r];
    if (choice !== 'default') reviewer_overrides[r] = choice === 'on';
  }
  return {
    enabled: values.enabled,
    skip_drafts: values.skip_drafts,
    skip_bots: values.skip_bots,
    target_branches: parseBranches(values.target_branches),
    reviewer_overrides,
  };
}
