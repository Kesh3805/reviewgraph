import type { MembershipRole } from './session';

/**
 * Role checks for rendering only: the API enforces every role. `member` is the "maintainer"
 * route role (enable, initialize, rebuild, settings, manual review, suppressions).
 */
export function canMaintain(role: MembershipRole | null | undefined): boolean {
  return role === 'member' || role === 'admin' || role === 'owner';
}

/** Organization administration (members, retention, model privacy). */
export function isAdmin(role: MembershipRole | null | undefined): boolean {
  return role === 'admin' || role === 'owner';
}
