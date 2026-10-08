/**
 * The `paths` type behind the typed API client.
 *
 * - `generated.ts` is produced from `packages/contracts/openapi/api.json` by
 *   `pnpm -F @reviewgraph/web gen:api` (never edit it by hand).
 * - `pending.ts` hand-writes the routes whose OpenAPI description does not exist yet (or is
 *   incomplete). Each entry there names the task that will publish it; once that task lands,
 *   regenerate, delete the entry here and in `pending.ts`, and the generated route takes over.
 */
import type { paths as GeneratedPaths } from './generated';
import type { PendingPaths } from './pending';

/** Generated routes whose response bodies are not described (yet) and are typed by hand. */
type Overridden = Extract<keyof PendingPaths, keyof GeneratedPaths>;

export type paths = Omit<GeneratedPaths, Overridden> & PendingPaths;
