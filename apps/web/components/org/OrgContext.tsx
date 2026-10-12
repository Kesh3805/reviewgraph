'use client';

import { createContext, useContext, type ReactNode } from 'react';
import type { MembershipRole } from '@/lib/session';

/** The organization the shell resolved from the session, plus the caller's role in it. */
export interface CurrentOrg {
  id: string;
  displayName: string;
  role: MembershipRole;
}

const OrgContext = createContext<CurrentOrg | null>(null);

export function OrgProvider({ org, children }: { org: CurrentOrg | null; children: ReactNode }) {
  return <OrgContext.Provider value={org}>{children}</OrgContext.Provider>;
}

/** The current organization, or null when the user has no membership. */
export function useCurrentOrg(): CurrentOrg | null {
  return useContext(OrgContext);
}
