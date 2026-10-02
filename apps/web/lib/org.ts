import type { Session, SessionOrganization } from './session';

/** Non-sensitive preference cookie holding the last selected organization id. */
export const ORG_COOKIE = 'rg_org';

/**
 * Resolves the active organization. The id always comes from the session memberships, so a
 * stale or tampered preference cookie can never select an organization the user is not in.
 */
export function resolveOrganization(
  session: Pick<Session, 'organizations' | 'current_organization_id'>,
  preferredId?: string,
): SessionOrganization | null {
  const byId = (id?: string | null) =>
    id ? session.organizations.find((org) => org.id === id) : undefined;
  return (
    byId(preferredId) ?? byId(session.current_organization_id) ?? session.organizations[0] ?? null
  );
}
