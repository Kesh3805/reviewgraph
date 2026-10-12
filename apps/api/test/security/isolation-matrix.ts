/**
 * Tenant isolation matrix (SEC-001): every API route and how isolation is proven for it. A route
 * missing here fails `every_route_has_isolation_matrix_entry`; add it with the behavior that
 * applies and, for `foreign_id_404`, make sure the two-organization suite covers its id params.
 *
 *   foreign_id_404     carries a tenant resource id; a foreign or unknown id answers 404
 *   list_filtered      lists tenant rows; never returns another organization's rows
 *   session_only       acts on the caller's own session or identity, no tenant data
 *   public             no session (health probes, OAuth redirects, signed webhooks)
 *   service_auth_only  `/internal/**`: service tokens only, user cookies rejected
 */
export type IsolationBehavior =
  'foreign_id_404' | 'list_filtered' | 'session_only' | 'public' | 'service_auth_only';

export const ISOLATION_MATRIX: Record<string, IsolationBehavior> = {
  'GET /health': 'public',
  'GET /api/v1/health/live': 'public',
  'GET /api/v1/health/ready': 'public',
  'GET /api/v1/auth/github/login': 'public',
  'GET /api/v1/auth/github/callback': 'public',
  'POST /api/v1/auth/logout': 'session_only',
  'GET /api/v1/auth/me': 'session_only',
  'POST /api/v1/webhooks/github': 'public',

  'POST /api/v1/repositories': 'foreign_id_404',
  'GET /api/v1/repositories': 'list_filtered',
  'GET /api/v1/repositories/{repoId}': 'foreign_id_404',
  'POST /api/v1/repositories/{repoId}/initialize': 'foreign_id_404',
  'GET /api/v1/repositories/{repoId}/status': 'foreign_id_404',
  'GET /api/v1/repositories/{repoId}/profile': 'foreign_id_404',
  'POST /api/v1/repositories/{repoId}/graph/rebuild': 'foreign_id_404',
  'PATCH /api/v1/repositories/{repoId}/settings': 'foreign_id_404',

  'GET /api/v1/repositories/{repoId}/pull-requests': 'foreign_id_404',
  'GET /api/v1/pull-requests/{pullRequestId}': 'foreign_id_404',
  'POST /api/v1/pull-requests/{pullRequestId}/review': 'foreign_id_404',
  'GET /api/v1/pull-requests/{pullRequestId}/reviews': 'foreign_id_404',
  'GET /api/v1/pull-requests/{pullRequestId}/reviews/{reviewId}': 'foreign_id_404',
  'POST /api/v1/reviews/{reviewId}/cancel': 'foreign_id_404',

  'GET /api/v1/reviews/{reviewId}/findings': 'foreign_id_404',
  'GET /api/v1/findings/{findingId}': 'foreign_id_404',
  'GET /api/v1/findings/{findingId}/trace': 'foreign_id_404',
  'POST /api/v1/findings/{findingId}/feedback': 'foreign_id_404',
  'GET /api/v1/findings/{findingId}/feedback': 'foreign_id_404',
  'GET /api/v1/repositories/{repoId}/feedback/summary': 'foreign_id_404',

  'GET /api/v1/repositories/{repoId}/graph/symbols': 'foreign_id_404',
  'GET /api/v1/repositories/{repoId}/graph/symbols/{key}': 'foreign_id_404',
  'GET /api/v1/repositories/{repoId}/graph/symbols/{key}/neighbors': 'foreign_id_404',
  'POST /api/v1/repositories/{repoId}/graph/subgraph': 'foreign_id_404',
  'GET /api/v1/repositories/{repoId}/graph/path': 'foreign_id_404',
  'GET /api/v1/reviews/{reviewId}/impact/{symbolKey}': 'foreign_id_404',
  'GET /api/v1/repositories/{repoId}/source': 'foreign_id_404',

  'GET /api/v1/organizations/{organizationId}/audit': 'foreign_id_404',
  'GET /api/v1/organizations/{organizationId}/audit/verify': 'foreign_id_404',
};

/** Path params that carry a tenant resource id (the rest, like `{key}`, are opaque). */
export const TENANT_ID_PARAMS = [
  'repoId',
  'pullRequestId',
  'reviewId',
  'findingId',
  'organizationId',
] as const;
export type TenantIdParam = (typeof TENANT_ID_PARAMS)[number];

/** Fills a matrix path with ids (and `k1` for opaque params). */
export function fillPath(path: string, ids: Record<TenantIdParam, string>): string {
  return path.replace(/\{(\w+)\}/g, (_m, name: string) =>
    (TENANT_ID_PARAMS as readonly string[]).includes(name) ? ids[name as TenantIdParam] : 'k1',
  );
}
