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
    items: [{ href: '/', labelKey: 'nav.drawer.log' }],
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
      if ((pathname === item.href || pathname.startsWith(`${item.href}/`)) && (!best || item.href.length > best.length)) {
        best = item.href;
      }
    }
  }
  return best ?? (pathname === '/' ? '/' : undefined);
}
