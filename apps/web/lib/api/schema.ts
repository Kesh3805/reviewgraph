/**
 * Hand-written stand-in for the generated OpenAPI types. Once API-008 publishes
 * `packages/contracts/openapi/api.json`, run `pnpm --filter @reviewgraph/web gen:api` and
 * switch `api-client.ts` to import `paths` from the generated `schema.d.ts`.
 */
import type { Session } from '../session';

export interface paths {
  '/api/v1/auth/me': {
    get: {
      responses: {
        200: { content: { 'application/json': Session } };
      };
    };
  };
  '/api/v1/auth/logout': {
    post: {
      responses: {
        204: { content: never };
      };
    };
  };
}
