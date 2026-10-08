import { cache } from 'react';
import { cookies } from 'next/headers';

export { SESSION_COOKIE } from './constants';

export interface SessionUser {
  id: string;
  login: string;
  display_name: string | null;
  avatar_url: string | null;
}

/**
 * Stored membership roles (API-003). `member` is the "maintainer" route role: it may enable,
 * initialize and configure repositories; `admin` and `owner` also manage the organization.
 */
export type MembershipRole = 'owner' | 'admin' | 'member' | 'viewer';

export interface SessionOrganization {
  id: string;
  slug: string;
  display_name: string;
  role: MembershipRole;
}

/**
 * Shape of `GET /auth/me` (API-004). The OpenAPI document does not describe this response
 * yet, so it is typed by hand from `auth.controller.ts`.
 */
export interface Session {
  user: SessionUser;
  organizations: SessionOrganization[];
  /** Not returned by the API today; kept optional for a future server-side preference. */
  current_organization_id?: string | null;
}

export function apiInternalUrl(): string {
  return process.env.API_INTERNAL_URL ?? 'http://127.0.0.1:8080';
}

/**
 * Resolves the current session from a server component by calling `GET /auth/me` with the
 * request cookies forwarded. Returns null when there is no (valid) session; any other failure
 * throws so the error boundary renders.
 */
export const getSession = cache(async (): Promise<Session | null> => {
  const cookieHeader = (await cookies()).toString();
  if (!cookieHeader) return null;
  const res = await fetchSession(cookieHeader);
  return res;
});

export async function fetchSession(
  cookieHeader: string,
  fetchImpl: typeof fetch = fetch,
  baseUrl = apiInternalUrl(),
): Promise<Session | null> {
  const res = await fetchImpl(`${baseUrl}/api/v1/auth/me`, {
    headers: { cookie: cookieHeader, accept: 'application/json' },
    cache: 'no-store',
  });
  if (res.status === 401 || res.status === 403) return null;
  if (!res.ok) throw new Error(`session lookup failed with status ${res.status}`);
  return (await res.json()) as Session;
}
