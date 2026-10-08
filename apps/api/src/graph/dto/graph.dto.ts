import { createZodDto } from 'nestjs-zod';
import { z } from 'zod';

/** Budgets enforced before the engine is called (the engine enforces them again). */
export const MAX_SUBGRAPH_NODES = 500;
export const MAX_SUBGRAPH_DEPTH = 3;
export const MAX_PATH_DEPTH = 6;
export const MAX_EXCERPT_LINES = 200;
export const MAX_SYMBOL_RESULTS = 50;

const snapshot = z.string().uuid().optional();
const kindList = z
  .string()
  .max(500)
  .transform((v) => v.split(',').filter(Boolean))
  .pipe(z.array(z.string().regex(/^[A-Z_]{1,40}$/i)).max(40));

export const SymbolSearchQuerySchema = z.object({
  q: z.string().min(1).max(200).optional(),
  kind: z
    .string()
    .regex(/^[A-Za-z_]{1,40}$/)
    .optional(),
  snapshot,
  limit: z.coerce.number().int().min(1).max(MAX_SYMBOL_RESULTS).default(20),
});

export const SnapshotQuerySchema = z.object({ snapshot });

export const NeighborsQuerySchema = z.object({
  dir: z.enum(['in', 'out']).default('out'),
  /** Comma separated edge kinds, e.g. `CALLS,IMPORTS`. */
  kinds: kindList.optional(),
  min_confidence: z.coerce.number().min(0).max(1).optional(),
  snapshot,
});

export const SubgraphRequestSchema = z
  .object({
    seeds: z.array(z.string().min(1).max(500)).min(1).max(50),
    depth: z.number().int().min(1).max(MAX_SUBGRAPH_DEPTH).default(1),
    kinds: z
      .array(z.string().regex(/^[A-Z_]{1,40}$/i))
      .max(40)
      .optional(),
    max_nodes: z.number().int().min(1).max(MAX_SUBGRAPH_NODES).default(200),
    snapshot,
  })
  .strict();

export const PathQuerySchema = z.object({
  from: z.string().min(1).max(500),
  to: z.string().min(1).max(500),
  max_depth: z.coerce.number().int().min(1).max(MAX_PATH_DEPTH).default(4),
  snapshot,
});

export const SourceQuerySchema = z
  .object({
    path: z
      .string()
      .min(1)
      .max(1000)
      .refine(
        (p) => !p.split('/').includes('..') && !p.startsWith('/'),
        'must be repository-relative',
      ),
    start: z.coerce.number().int().min(1),
    end: z.coerce.number().int().min(1),
    snapshot,
  })
  .refine((q) => q.end >= q.start, { message: 'end must not be before start', path: ['end'] })
  .refine((q) => q.end - q.start + 1 <= MAX_EXCERPT_LINES, {
    message: `at most ${MAX_EXCERPT_LINES} lines`,
    path: ['end'],
  });

/** Graph responses are the engine's (API-013 DTOs); budget truncation is passed through. */
export const GraphResponseSchema = z
  .object({
    snapshot_id: z.string(),
    truncated: z.boolean(),
  })
  .catchall(z.unknown());

export const SourceExcerptSchema = z.object({
  snapshot_id: z.string(),
  path: z.string(),
  start: z.number().int(),
  end: z.number().int(),
  /** Redacted text, at most 200 lines. */
  text: z.string(),
  redacted: z.boolean(),
  truncated: z.boolean(),
});

export class SymbolSearchQueryDto extends createZodDto(SymbolSearchQuerySchema) {}
export class SnapshotQueryDto extends createZodDto(SnapshotQuerySchema) {}
export class NeighborsQueryDto extends createZodDto(NeighborsQuerySchema) {}
export class SubgraphRequestDto extends createZodDto(SubgraphRequestSchema) {}
export class PathQueryDto extends createZodDto(PathQuerySchema) {}
export class SourceQueryDto extends createZodDto(SourceQuerySchema) {}
export class GraphResponseDto extends createZodDto(GraphResponseSchema) {}
export class SourceExcerptDto extends createZodDto(SourceExcerptSchema) {}

export type SymbolSearchQuery = z.infer<typeof SymbolSearchQuerySchema>;
export type NeighborsQuery = z.infer<typeof NeighborsQuerySchema>;
export type SubgraphRequest = z.infer<typeof SubgraphRequestSchema>;
export type PathQuery = z.infer<typeof PathQuerySchema>;
export type SourceQuery = z.infer<typeof SourceQuerySchema>;
export type GraphResponse = z.infer<typeof GraphResponseSchema>;
export type SourceExcerpt = z.infer<typeof SourceExcerptSchema>;
