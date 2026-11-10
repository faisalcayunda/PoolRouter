import { createContext, useContext, type ReactNode } from 'react';

/**
 * Admin role of the signed-in user (ADR-0003 rev 5 3.6c: `platform_admin`, `operator`, `auditor`, plus `client_owner`, who is bound to one client).
 * Filled by the session hook of the shell; screens never guess it. Until the session lands the
 * value is `null`, which renders as "no role-specific affordances" — the API stays the last guard
 * for every role-gated action (handoff spec section 18.1).
 */
export type AdminRole = 'platform_admin' | 'operator' | 'auditor' | 'client_owner';

export const RoleContext = createContext<AdminRole | null>(null);

export function useRole(): AdminRole | null {
  return useContext(RoleContext);
}

/**
 * `current` rather than `role`: a JSX prop literally named `role` is read as an ARIA attribute by
 * the a11y lint, and this is a context value, not a role.
 */
export function RoleProvider({
  current,
  children,
}: {
  current: AdminRole | null;
  children: ReactNode;
}) {
  return <RoleContext.Provider value={current}>{children}</RoleContext.Provider>;
}
