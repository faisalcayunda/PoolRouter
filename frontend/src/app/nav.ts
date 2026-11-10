import type { AdminRole } from '../features/common/RoleContext';
import type { icons } from '../icons';

type IconKey = keyof typeof icons;

/** Navigation model of the app shell (ADR-0003 5.1, UX v0.3 navigation). */
export type NavGroupId = 'monitoring' | 'config' | 'client' | 'compliance' | 'account';

export interface NavItem {
  /** Route path of the F0 screen. */
  href: string;
  /** i18n key under `nav.drawer.*`; never a hardcoded label. */
  labelKey: string;
  /** Kunci ikon (`icons`), 24 px di depan label (desain: `Icon / <nama>` per item). */
  icon: IconKey;
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
 * Isi drawer berurutan. Per peran ini mengikuti frame Penpot
 * `F0 / Shell-platform-admin|operator|auditor` (`.takeover/design/shell-v2-spec.md` bagian 2):
 * - platform_admin: Kuota; Provider, Secret, Upstream pool, Downstream pool; Client dan kredensial; Pengaturan.
 * - operator: Log transaksi, Kuota; Pengaturan.
 * - auditor: Log transaksi; Audit akses, Pencarian log per NIK; Pengaturan.
 * `client_owner` tidak punya frame; ia melihat yang diizinkan layarnya (`canViewClients`) plus Pengaturan.
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    id: 'monitoring',
    labelKey: 'nav.group.monitoring',
    items: [
      {
        href: '/',
        labelKey: 'nav.drawer.log',
        icon: 'receiptLong',
        roles: ['operator', 'auditor'],
      },
      {
        href: '/kuota',
        labelKey: 'nav.drawer.quota',
        icon: 'dataUsage',
        roles: ['platform_admin', 'operator'],
      },
    ],
  },
  {
    id: 'config',
    labelKey: 'nav.group.config',
    items: [
      {
        href: '/provider',
        labelKey: 'nav.drawer.provider',
        icon: 'dns',
        roles: ['platform_admin'],
      },
      {
        href: '/endpoint',
        labelKey: 'nav.drawer.endpoint',
        icon: 'api',
        roles: ['platform_admin'],
      },
      { href: '/secret', labelKey: 'nav.drawer.secret', icon: 'key', roles: ['platform_admin'] },
      {
        href: '/upstream',
        labelKey: 'nav.drawer.upstream',
        icon: 'cloudUpload',
        roles: ['platform_admin'],
      },
      {
        href: '/downstream',
        labelKey: 'nav.drawer.downstream',
        icon: 'cloudDownload',
        roles: ['platform_admin'],
      },
    ],
  },
  {
    id: 'client',
    labelKey: 'nav.group.client',
    items: [
      {
        href: '/client',
        labelKey: 'nav.drawer.client',
        icon: 'apps',
        roles: ['platform_admin', 'client_owner'],
      },
    ],
  },
  {
    id: 'compliance',
    labelKey: 'nav.group.compliance',
    items: [
      { href: '/audit', labelKey: 'nav.drawer.audit', icon: 'factCheck', roles: ['auditor'] },
      {
        href: '/nik',
        labelKey: 'nav.drawer.nik_search',
        icon: 'personSearch',
        roles: ['auditor'],
      },
    ],
  },
  {
    id: 'account',
    labelKey: 'nav.group.account',
    items: [{ href: '/settings', labelKey: 'nav.drawer.settings', icon: 'settings' }],
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
  if (best) return best;
  return pathname === '/' || pathname.startsWith('/log/') ? '/' : undefined;
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

function crumbsForItem(href: string, tail: Crumb[] = []): Crumb[] {
  for (const group of NAV_GROUPS) {
    const item = group.items.find((i) => i.href === href);
    if (item) {
      return [
        { labelKey: group.labelKey },
        tail.length > 0 ? { labelKey: item.labelKey, href } : { labelKey: item.labelKey },
        ...tail,
      ];
    }
  }
  return [];
}

/** Rute detail di bawah item daftar: `/<segmen>/<id>`; `baru` pada rute form berarti "baru". */
const DETAIL_BASE: Record<string, { base: string; hasNew: boolean }> = {
  log: { base: '/', hasNew: false },
  provider: { base: '/provider', hasNew: true },
  endpoint: { base: '/endpoint', hasNew: true },
  upstream: { base: '/upstream', hasNew: true },
  downstream: { base: '/downstream', hasNew: true },
  client: { base: '/client', hasNew: false },
};
const DETAIL =
  /^\/(log|provider|endpoint|upstream|downstream|client)\/([^/]+)(?:\/(kredensial|webhook))?$/;

/**
 * Jejak breadcrumb untuk setiap rute yang dirender shell, termasuk detail dan not-found.
 * Grup berupa teks biasa, daftar menjadi tautan di rute detail, crumb terakhir adalah halaman
 * saat ini (desain: `Shell-detail`). Crumb terakhir sebuah record adalah id dari URL karena shell
 * tidak mengetahui nama record.
 */
export function crumbsFor(pathname: string): Crumb[] {
  const clean = pathname.replace(/\/+$/, '') || '/';
  const detail = DETAIL.exec(clean);
  if (detail) {
    const segment = detail[1] ?? '';
    const target = DETAIL_BASE[segment];
    const id = decodeURIComponent(detail[2] ?? '');
    const sub = detail[3];
    if (target && (!sub || segment === 'client')) {
      if (sub) {
        const idHref = `/client/${encodeURIComponent(id)}`;
        return crumbsForItem(target.base, [
          { text: id, href: idHref },
          { labelKey: sub === 'kredensial' ? 'nav.breadcrumb.credentials' : 'nav.drawer.webhook' },
        ]);
      }
      return crumbsForItem(target.base, [
        id === 'baru' && target.hasNew ? { labelKey: 'nav.breadcrumb.new' } : { text: id },
      ]);
    }
  }
  const own = crumbsForItem(clean);
  return own.length > 0 ? own : [{ labelKey: 'nav.breadcrumb.not_found' }];
}
