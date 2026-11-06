/** Navigation model of the app shell (ADR-0003 5.1, UX v0.3 navigation). */
export type NavGroupId = 'monitoring' | 'config' | 'client' | 'compliance' | 'account';

export interface NavItem {
  /** Route path of the F0 screen. */
  href: string;
  /** i18n key under `nav.drawer.*`; never a hardcoded label. */
  labelKey: string;
}

export interface NavGroup {
  id: NavGroupId;
  /** i18n key under `nav.group.*`. */
  labelKey: string;
  items: NavItem[];
}

/**
 * Drawer contents in display order. Only screens that exist in F0 are listed;
 * SHAA-202/203 add their routes here when they land.
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    id: 'monitoring',
    labelKey: 'nav.group.monitoring',
    items: [
      { href: '/', labelKey: 'nav.drawer.log' },
      { href: '/kuota', labelKey: 'nav.drawer.quota' },
    ],
  },
  {
    id: 'config',
    labelKey: 'nav.group.config',
    items: [
      { href: '/provider', labelKey: 'nav.drawer.provider' },
      { href: '/secret', labelKey: 'nav.drawer.secret' },
      { href: '/upstream', labelKey: 'nav.drawer.upstream' },
      { href: '/downstream', labelKey: 'nav.drawer.downstream' },
    ],
  },
  {
    id: 'client',
    labelKey: 'nav.group.client',
    items: [{ href: '/client', labelKey: 'nav.drawer.client' }],
  },
  {
    id: 'compliance',
    labelKey: 'nav.group.compliance',
    items: [
      { href: '/audit', labelKey: 'nav.drawer.audit' },
      { href: '/nik', labelKey: 'nav.drawer.nik_search' },
    ],
  },
  {
    id: 'account',
    labelKey: 'nav.group.account',
    items: [{ href: '/settings', labelKey: 'nav.drawer.settings' }],
  },
];

/** The item whose `href` is the longest prefix of `pathname` is the active one. */
export function activeHref(pathname: string): string | undefined {
  let best: string | undefined;
  for (const group of NAV_GROUPS) {
    for (const item of group.items) {
      if (item.href === '/') continue;
      if (
        (pathname === item.href || pathname.startsWith(`${item.href}/`)) &&
        (!best || item.href.length > best.length)
      ) {
        best = item.href;
      }
    }
  }
  return best ?? (pathname === '/' ? '/' : undefined);
}

import type { AdminRole } from '../features/common/RoleContext';

/** Groups each role may see in the drawer (handoff ui-v2-layar, section 2). */
const ROLE_GROUPS: Record<AdminRole, ReadonlyArray<NavGroupId>> = {
  platform_admin: ['monitoring', 'config', 'client', 'compliance', 'account'],
  operator: ['monitoring', 'compliance', 'account'],
  auditor: ['compliance', 'account'],
  client_owner: ['client', 'account'],
};

/** Returns the nav groups visible to `role`. `null` (no session) shows everything. */
export function visibleNavGroups(role: AdminRole | null): NavGroup[] {
  if (role === null) return NAV_GROUPS;
  const allowed = new Set(ROLE_GROUPS[role]);
  return NAV_GROUPS.filter((g) => allowed.has(g.id));
}

export const filterNavByRole = visibleNavGroups;
export const navForRole = visibleNavGroups;
export const filterMenuByRole = visibleNavGroups;
