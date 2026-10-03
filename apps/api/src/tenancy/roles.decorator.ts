import { SetMetadata } from '@nestjs/common';
import type { MembershipRole } from './request-context';

/**
 * Route roles. `maintainer` is the stored `member` role (the schema role set is
 * owner/admin/member/viewer); anything above it also satisfies it.
 */
export type RequiredRole = 'viewer' | 'maintainer' | 'admin';

export const ROLE_RANK: Record<MembershipRole, number> = {
  viewer: 0,
  member: 1,
  admin: 2,
  owner: 3,
};

const REQUIRED_RANK: Record<RequiredRole, number> = { viewer: 0, maintainer: 1, admin: 2 };

export const REQUIRE_ROLE_KEY = 'rg:require-role';

/** Marks a route as tenant scoped and sets the minimum role. See TenancyGuard. */
export const RequireRole = (role: RequiredRole): MethodDecorator & ClassDecorator =>
  SetMetadata(REQUIRE_ROLE_KEY, role);

export function satisfies(actual: MembershipRole, required: RequiredRole): boolean {
  return ROLE_RANK[actual] >= REQUIRED_RANK[required];
}
