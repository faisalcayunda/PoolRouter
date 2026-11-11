import { Suspense, useCallback, useEffect, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Button as RacButton, Link } from 'react-aria-components';
import { Breadcrumbs } from '../components/m3/Breadcrumbs';
import { cx } from '../components/m3/cx';
import { IconButton } from '../components/m3/IconButton';
import { Menu, MenuItem, MenuTrigger } from '../components/m3/Menu';
import { NavigationDrawer } from '../components/m3/NavigationDrawer';
import { TopAppBar } from '../components/m3/TopAppBar';
import { enumLabel } from '../i18n/enums';
import { Icon, icons } from '../icons';
import { CrumbNameProvider, type CrumbRecord } from '../features/common/CrumbName';
import type { AdminRole } from '../features/common/RoleContext';
import { activeHref, crumbsFor, navGroupsFor } from './nav';

/**
 * Drawer standar hanya dari kelas jendela M3 `large` (>= 1200 px, `--breakpoint-large` di
 * tailwind.css). Desain menggambar 1024, 768 dan 375 sebagai drawer modal dan hanya 1440 sebagai
 * standar; batas persisnya inferensi (`.takeover/design/shell-v2-spec.md` bagian 3).
 */
const LARGE_QUERY = '(min-width: 1200px)';

function useIsLarge(): boolean {
  const [large, setLarge] = useState(
    () => typeof window !== 'undefined' && window.matchMedia(LARGE_QUERY).matches,
  );
  useEffect(() => {
    const mql = window.matchMedia(LARGE_QUERY);
    const onChange = (e: MediaQueryListEvent) => setLarge(e.matches);
    mql.addEventListener('change', onChange);
    return () => mql.removeEventListener('change', onChange);
  }, []);
  return large;
}

/** Logomark kecil: dua jalur bertemu di satu keluaran (router). Dekoratif; nama produk tertulis di sampingnya. */
function LogoMark() {
  return (
    <span
      aria-hidden="true"
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-small bg-primary text-on-primary"
    >
      <svg
        viewBox="0 0 24 24"
        width="18"
        height="18"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M5 6h5a4 4 0 0 1 4 4v8" />
        <path d="M5 18h5a4 4 0 0 0 2.6-1" />
        <path d="m11 15 3 3 3-3" />
      </svg>
    </span>
  );
}

function initialOf(username: string): string {
  return username.trim().charAt(0).toUpperCase();
}

/**
 * Menu akun: satu-satunya jalan ke "Keluar". Dua pemicu berbagi menu yang sama: blok identitas di
 * dasar sidebar (`block`, atau hanya lingkaran inisial saat sidebar diciutkan) dan avatar di bar atas.
 */
function AccountMenu({
  user,
  onKeluar,
  variant,
  compact = false,
}: {
  user: NonNullable<AppShellProps['user']>;
  onKeluar?: () => void;
  variant: 'block' | 'avatar';
  compact?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const initial = (
    <span
      aria-hidden="true"
      className="typescale-label-large flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-accent-container text-on-secondary-container"
    >
      {initialOf(user.username)}
    </span>
  );
  return (
    <MenuTrigger>
      <RacButton
        aria-label={t('account.menu.menu', { nama_pengguna: user.username })}
        className={cx(
          'cursor-pointer outline-none focus-visible:outline-solid focus-visible:outline-3 focus-visible:outline-offset-2 focus-visible:outline-secondary',
          variant === 'avatar'
            ? 'flex h-9 w-9 items-center justify-center rounded-full'
            : compact
              ? 'flex h-10 w-10 items-center justify-center self-center rounded-full'
              : 'flex w-full items-center gap-3 rounded-medium border border-outline-variant p-3 text-left hover:bg-surface-container-low',
        )}
      >
        {initial}
        {variant === 'block' && !compact ? (
          <span className="flex min-w-0 flex-col">
            <span className="typescale-label-large truncate">{user.username}</span>
            <span className="typescale-body-small text-on-surface-variant">
              {enumLabel(i18n, 'peran', user.peran)}
            </span>
          </span>
        ) : null}
      </RacButton>
      <Menu onAction={() => onKeluar?.()}>
        <MenuItem id="keluar">{t('account.menu.sign_out')}</MenuItem>
      </Menu>
    </MenuTrigger>
  );
}

/** Isi sidebar: logo + tombol ciut/tutup, grup menu per peran (grup akun di dasar), blok akun. */
function DrawerContent({
  pathname,
  role,
  user,
  onKeluar,
  onNavigate,
  onClose,
  collapsed = false,
  onToggleCollapsed,
}: {
  pathname: string;
  role: AdminRole | null;
  user?: AppShellProps['user'];
  onKeluar?: () => void;
  onNavigate?: () => void;
  /** Hanya varian modal: tombol "Tutup menu" di sudut kanan atas drawer. */
  onClose?: () => void;
  /** Hanya varian standard: sidebar diciutkan menjadi rel ikon. */
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
}) {
  const { t } = useTranslation();
  const current = activeHref(pathname);
  return (
    <>
      <div
        className={cx(
          'flex h-16 shrink-0 items-center border-b border-outline-variant',
          collapsed ? 'justify-center px-2' : 'justify-between gap-2 pr-3 pl-5',
        )}
      >
        {collapsed ? null : (
          <span className="flex min-w-0 items-center gap-2">
            <LogoMark />
            <span className="typescale-title-large truncate">{t('common.app.name')}</span>
          </span>
        )}
        {onClose ? (
          <IconButton aria-label={t('nav.drawer.close_menu')} onPress={onClose} variant="outlined">
            <Icon icon={icons.close} className="h-5 w-5" />
          </IconButton>
        ) : onToggleCollapsed ? (
          <IconButton
            aria-label={collapsed ? t('nav.drawer.expand_menu') : t('nav.drawer.collapse_menu')}
            onPress={onToggleCollapsed}
            variant="outlined"
          >
            <Icon
              icon={collapsed ? icons.leftPanelOpen : icons.leftPanelClose}
              className="h-5 w-5"
            />
          </IconButton>
        ) : null}
      </div>
      <nav
        aria-label={t('nav.drawer.aria')}
        className={cx(
          'flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto pt-4 pb-3',
          collapsed ? 'px-2' : 'px-4',
        )}
      >
        {navGroupsFor(role).map((group) => (
          <div
            key={group.id}
            className={cx('flex flex-col gap-1', group.id === 'account' && 'mt-auto')}
          >
            {collapsed ? (
              <hr aria-hidden="true" className="mx-2 mb-1 border-t border-outline-variant" />
            ) : (
              <p className="typescale-body-medium px-3 pb-1 text-on-surface-variant">
                {t(group.labelKey)}
              </p>
            )}
            {group.items.map((item) => {
              const isActive = item.href === current;
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  onPress={onNavigate}
                  aria-current={isActive ? 'page' : undefined}
                  {...(collapsed ? { title: t(item.labelKey) } : {})}
                  className={cx(
                    'typescale-body-medium flex h-10 items-center gap-3 rounded-small border px-3 text-on-surface no-underline outline-none',
                    collapsed && 'justify-center px-0',
                    isActive
                      ? 'border-outline-variant bg-surface-container-low font-semibold'
                      : 'border-transparent hover:bg-surface-container-low',
                    'focus-visible:outline-solid focus-visible:outline-3 focus-visible:outline-offset-2 focus-visible:outline-secondary',
                  )}
                >
                  <Icon
                    icon={icons[item.icon]}
                    className={cx('h-5 w-5 shrink-0', isActive ? 'text-on-surface' : undefined)}
                  />
                  <span className={collapsed ? 'sr-only' : undefined}>{t(item.labelKey)}</span>
                </Link>
              );
            })}
          </div>
        ))}
      </nav>
      {user ? (
        <div className={cx('flex shrink-0 flex-col pb-4', collapsed ? 'px-2' : 'px-4')}>
          <AccountMenu user={user} onKeluar={onKeluar} variant="block" compact={collapsed} />
        </div>
      ) : null}
    </>
  );
}

export interface AppShellProps {
  /** Current route path, used to mark the active drawer item. */
  pathname?: string;
  /** Identitas pengguna yang masuk, tampil di dasar drawer (desain: `Identitas / nama` dan `/ peran`). */
  user?: { username: string; peran: string } | null;
  /** Admin role that filters the drawer; `null` = unknown, full menu (API is the last guard). */
  role?: AdminRole | null;
  /** Dipanggil dari item "Keluar" di menu akun (dasar drawer). */
  onKeluar?: () => void;
  /** Screen rendered inside `<main>`. */
  children?: ReactNode;
}

/**
 * Application shell (acuan ARC-261, `.takeover/design/arc-261/spec.md`): link lompat, sidebar
 * putih, bar atas 64 px berisi breadcrumb, pencarian dekoratif, label lingkungan dan avatar, lalu
 * konten di panel slate-50.
 *
 * - large (>= 1200 px): `NavigationDrawer variant="standard"`, selalu tampil, tinggi penuh; bar atas
 *   dan konten ada di kanannya.
 * - di bawahnya (1024, 768, 375): `variant="modal"` dengan scrim, dibuka dari tombol menu di bar atas.
 */
export function AppShell({ pathname = '/', user, role = null, onKeluar, children }: AppShellProps) {
  const { t } = useTranslation();
  const isLarge = useIsLarge();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [collapsedPref, setCollapsed] = useState(false);
  const closeDrawer = useCallback(() => setDrawerOpen(false), []);
  const [record, setRecord] = useState<CrumbRecord | null>(null);

  // Drawer modal hanya ada di bawah large; diturunkan dari `isLarge` supaya tidak perlu efek
  // sinkronisasi dan tidak pernah tersisa terbuka setelah jendela diperlebar. Ciut hanya berlaku
  // pada sidebar standard.
  const isModalDrawerOpen = drawerOpen && !isLarge;
  const collapsed = collapsedPref && isLarge;

  return (
    <div className="flex min-h-dvh bg-surface-container-lowest text-on-surface">
      <a
        href="#main-content"
        className="typescale-label-large sr-only rounded-small bg-primary px-4 py-2 text-on-primary focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50"
      >
        {t('nav.skip_link')}
      </a>

      <NavigationDrawer
        variant={isLarge ? 'standard' : 'modal'}
        aria-label={t('nav.drawer.aria')}
        isOpen={isModalDrawerOpen}
        onOpenChange={(open) => setDrawerOpen(open)}
        collapsed={collapsed}
      >
        <DrawerContent
          pathname={pathname}
          role={role}
          user={user}
          onKeluar={onKeluar}
          onNavigate={isLarge ? undefined : closeDrawer}
          onClose={isLarge ? undefined : closeDrawer}
          collapsed={collapsed}
          onToggleCollapsed={isLarge ? () => setCollapsed((c) => !c) : undefined}
        />
      </NavigationDrawer>

      {/* Panel konten: latar slate-50, sudut kiri atas membulat besar di sebelah sidebar. */}
      <div
        className={cx(
          'flex min-w-0 flex-1 flex-col bg-surface-container-low',
          isLarge && 'rounded-tl-extra-large border-t border-l border-outline-variant',
        )}
      >
        <TopAppBar
          className="px-6"
          leading={
            isLarge ? undefined : (
              <IconButton
                aria-label={t('nav.drawer.open_menu')}
                onPress={() => setDrawerOpen(true)}
                className="-ml-3"
              >
                <Icon icon={icons.menu} />
              </IconButton>
            )
          }
          title={
            <Breadcrumbs
              aria-label={t('nav.breadcrumb.aria')}
              items={crumbsFor(pathname, record).map((c) => ({
                label: c.labelKey ? t(c.labelKey) : c.text,
                ...(c.href ? { href: c.href } : {}),
              }))}
            />
          }
          trailing={
            <>
              {/* Kolom pencarian hanya gaya acuan: belum ada fitur pencarian global, jadi dekoratif dan tidak fokus. */}
              <div
                aria-hidden="true"
                className="hidden h-9 w-64 items-center gap-2 rounded-small border border-outline-variant bg-surface-container-lowest px-3 text-on-surface-variant expanded:flex"
              >
                <Icon icon={icons.search} className="h-5 w-5 shrink-0" />
                <input
                  disabled
                  tabIndex={-1}
                  placeholder={t('common.action.search')}
                  className="typescale-body-medium min-w-0 flex-1 bg-transparent outline-none placeholder:text-on-surface-variant"
                />
              </div>
              <span className="typescale-label-medium rounded-full border border-outline-variant bg-surface-container-lowest px-3 py-1 text-on-surface">
                {t('nav.env_label')}
              </span>
              {user ? <AccountMenu user={user} onKeluar={onKeluar} variant="avatar" /> : null}
            </>
          }
        />

        <main id="main-content" tabIndex={-1} className="min-w-0 flex-1 px-6 pt-4 pb-8">
          {/* Konten dibatasi 1200 px (spec bagian 4) supaya tabel panjang tetap terbaca di layar lebar. */}
          <div className="mx-auto flex w-full max-w-[1200px] flex-col gap-4">
            <Suspense
              fallback={<p className="typescale-body-medium">{t('common.state.loading')}</p>}
            >
              <CrumbNameProvider onChange={setRecord}>{children}</CrumbNameProvider>
            </Suspense>
          </div>
        </main>
      </div>
    </div>
  );
}
