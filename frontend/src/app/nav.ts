import type { AdminRole } from '../features/common/RoleContext';

/** Navigation model of the app shell (ADR-0003 5.1, UX v0.3 navigation). */
export type NavGroupId = 'monitoring' | 'config' | 'client' | 'compliance' | 'account';

export interface NavItem {
  /** Route path of the F0 screen. */
  href: string;
  /** i18n key under `nav.drawer.*`; never a hardcoded label. */
  labelKey: string;
  /** Roles that see the item; omitted = every role. The API stays the last guard (spec 18.1). */
  roles?: readonly AdminRole[];
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
      { href: '/kuota', labelKey: 'nav.drawer.quota', roles: ['platform_admin', 'operator'] },
    ],
  },
  {
    id: 'config',
    labelKey: 'nav.group.config',
    items: [
      { href: '/provider', labelKey: 'nav.drawer.provider', roles: ['platform_admin'] },
      { href: '/secret', labelKey: 'nav.drawer.secret', roles: ['platform_admin'] },
      {
        href: '/upstream',
        labelKey: 'nav.drawer.upstream',
        roles: ['platform_admin', 'operator'],
      },
      {
        href: '/downstream',
        labelKey: 'nav.drawer.downstream',
        roles: ['platform_admin', 'operator'],
      },
    ],
  },
  {
    id: 'client',
    labelKey: 'nav.group.client',
    items: [
      { href: '/client', labelKey: 'nav.drawer.client', roles: ['platform_admin', 'operator'] },
    ],
  },
  {
    id: 'compliance',
    labelKey: 'nav.group.compliance',
    items: [
      { href: '/audit', labelKey: 'nav.drawer.audit', roles: ['platform_admin', 'auditor'] },
      { href: '/nik', labelKey: 'nav.drawer.nik_search', roles: ['auditor'] },
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

/**
 * Drawer contents for one role; groups left empty are dropped. `null` (session role unknown) keeps
 * the full menu, the same way the screens treat `null`: the API refuses what the role cannot do.
 */
export function navGroupsFor(role: AdminRole | null): NavGroup[] {
  return NAV_GROUPS.map((group) => ({
    ...group,
    items: group.items.filter((item) => role === null || !item.roles || item.roles.includes(role)),
  })).filter((group) => group.items.length > 0);
}

export interface Crumb {
  /** i18n key, or literal text for a dynamic segment (record id or transaction code). */
  labelKey?: string;
  text?: string;
  href?: string;
}

function crumbsForItem(href: string, tail?: Crumb): Crumb[] {
  for (const group of NAV_GROUPS) {
    const item = group.items.find((i) => i.href === href);
    if (item) {
      return [
        { labelKey: group.labelKey },
        tail ? { labelKey: item.labelKey, href } : { labelKey: item.labelKey },
        ...(tail ? [tail] : []),
      ];
    }
  }
  return [];
}

/** Breadcrumb trail for every route the shell renders, including detail and not-found paths. */
export function crumbsFor(pathname: string): Crumb[] {
  const clean = pathname.replace(/\/+$/, '') || '/';
  const detail = /^\/(log|provider)\/([^/]+)$/.exec(clean);
  if (detail) {
    const base = detail[1] === 'log' ? '/' : '/provider';
    const id = decodeURIComponent(detail[2] ?? '');
    return crumbsForItem(
      base,
      id === 'baru' && detail[1] === 'provider' ? { labelKey: 'nav.breadcrumb.new' } : { text: id },
    );
  }
  const own = crumbsForItem(clean);
  return own.length > 0 ? own : [{ labelKey: 'nav.breadcrumb.not_found' }];
}
