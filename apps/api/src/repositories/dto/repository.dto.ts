import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';

/** The six reviewers a repository can toggle (mirrors the `reviewer_runs.reviewer` values). */
export const REVIEWER_TYPES = [
  'correctness',
  'security',
  'test',
  'architecture',
  'performance',
  'maintainability',
] as const;

const uuid = z.string().uuid();

export const RepositorySettingsSchema = z.object({
  enabled: z.boolean(),
  /** Base-branch patterns; empty means every branch. A trailing `*` is a prefix match. */
  target_branches: z.array(z.string()),
  skip_drafts: z.boolean(),
  skip_bots: z.boolean(),
  /** Reviewer toggles over the policy defaults. */
  reviewer_overrides: z.partialRecord(z.enum(REVIEWER_TYPES), z.boolean()),
});

export const RepositorySchema = z.object({
  id: uuid,
  organization_id: uuid,
  provider: z.literal('github'),
  provider_repo_id: z.string(),
  full_name: z.string(),
  default_branch: z.string(),
  visibility: z.enum(['public', 'private', 'internal']),
  archived: z.boolean(),
  /** False once the repository lost access or its installation was removed. */
  enabled: z.boolean(),
  access_state: z.enum(['active', 'removed', 'access_lost', 'installation_deleted']),
  primary_language: z.string().nullable(),
  initialized_at: z.string().nullable(),
  settings: RepositorySettingsSchema,
  created_at: z.string(),
  updated_at: z.string(),
});

export const RepositoryListSchema = z.object({
  items: z.array(RepositorySchema),
  /** Opaque; pass it back as `cursor` for the next page. Null on the last page. */
  next_cursor: z.string().nullable(),
});

const branchPattern = z
  .string()
  .min(1)
  .max(255)
  .regex(/^[^\s]+$/, 'must not contain whitespace');

export const CreateRepositorySchema = z
  .object({
    /** The installation (`provider_installations.id`) the repository belongs to. */
    installation_id: uuid,
    /** `owner/name` as the provider reports it. */
    full_name: z
      .string()
      .min(3)
      .max(300)
      .regex(/^[^/\s]+\/[^/\s]+$/, 'expected owner/name'),
  })
  .strict();

export const UpdateRepositorySettingsSchema = z
  .object({
    enabled: z.boolean(),
    target_branches: z.array(branchPattern).max(50),
    skip_drafts: z.boolean(),
    skip_bots: z.boolean(),
    reviewer_overrides: z.partialRecord(z.enum(REVIEWER_TYPES), z.boolean()),
  })
  .partial()
  .strict()
  .refine((patch) => Object.keys(patch).length > 0, 'at least one setting is required');

export const ListRepositoriesQuerySchema = z.object({
  organization_id: uuid.optional(),
  limit: z.coerce.number().int().min(1).max(100).default(50),
  cursor: z.string().max(500).optional(),
});

const nullableString = z.string().nullable();

/** `GET /repositories/:repoId/status` (API-008). */
export const RepositoryStatusSchema = z.object({
  repository_id: uuid,
  /** `not_initialized` until the first `review init`, `queued`/`indexing` while a job is active. */
  index_state: z.enum(['not_initialized', 'queued', 'indexing', 'ready']),
  /** The latest init facts (INIT-013): the repository fingerprint and the producing versions. */
  init: z
    .object({
      commit_sha: z.string(),
      fingerprint: nullableString,
      facts_hash: z.string(),
      facts_schema_version: z.number().int(),
      tool_version: z.string(),
      primary_language: nullableString,
      is_monorepo: z.boolean(),
      frameworks: z.array(z.string()),
      warnings_count: z.number().int(),
      detected_at: z.string(),
    })
    .nullable(),
  /** Graph snapshots arrive with the graph tasks: null until one exists. */
  last_snapshot: z
    .object({
      full: z.object({ id: z.string(), commit_sha: z.string(), created_at: z.string() }).nullable(),
      delta: z
        .object({ id: z.string(), commit_sha: z.string(), created_at: z.string() })
        .nullable(),
    })
    .nullable(),
  active_job: z
    .object({ id: z.string(), queue: z.string(), state: z.enum(['queued', 'running']) })
    .nullable(),
  config: z.object({
    hash: nullableString,
    validation_errors: z.array(z.string()),
  }),
  profile_computed_at: nullableString,
});

export const IndexRequestSchema = z.object({
  job_id: z.string(),
  /** `repo-index:{repository_id}:{head_sha}` or `repo-rebuild:...`. */
  idempotency_key: z.string(),
  head_sha: z.string(),
});

export class RepositoryDto extends createZodDto(RepositorySchema) {}
export class RepositoryListDto extends createZodDto(RepositoryListSchema) {}
export class RepositorySettingsDto extends createZodDto(RepositorySettingsSchema) {}
export class CreateRepositoryDto extends createZodDto(CreateRepositorySchema) {}
export class UpdateRepositorySettingsDto extends createZodDto(UpdateRepositorySettingsSchema) {}
export class ListRepositoriesQueryDto extends createZodDto(ListRepositoriesQuerySchema) {}
export class RepositoryStatusDto extends createZodDto(RepositoryStatusSchema) {}
export class IndexRequestDto extends createZodDto(IndexRequestSchema) {}

export type RepositoryResponse = z.infer<typeof RepositorySchema>;
export type RepositoryStatusResponse = z.infer<typeof RepositoryStatusSchema>;
export type RepositorySettingsResponse = z.infer<typeof RepositorySettingsSchema>;
export type UpdateRepositorySettingsInput = z.infer<typeof UpdateRepositorySettingsSchema>;
