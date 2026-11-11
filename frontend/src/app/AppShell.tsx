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

/** Isi drawer: nama produk (dan tombol tutup di modal), grup menu per peran, identitas di dasar. */
function DrawerContent({
  pathname,
  role,
  user,
  onKeluar,
  onNavigate,
  onClose,
}: {
  pathname: string;
  role: AdminRole | null;
  user?: AppShellProps['user'];
  onKeluar?: () => void;
  onNavigate?: () => void;
  /** Hanya varian modal: tombol "Tutup menu" di sudut kanan atas drawer. */
  onClose?: () => void;
}) {
  const { t, i18n } = useTranslation();
  const current = activeHref(pathname);
  return (
    <>
      <div className="relative flex h-16 shrink-0 items-center px-7">
        <span className="typescale-title-large">{t('common.app.name')}</span>
        {onClose ? (
          <IconButton
            aria-label={t('nav.drawer.close_menu')}
            onPress={onClose}
            className="absolute top-2 right-3 outline-inverse-primary! data-[focus-visible]:outline-inverse-primary!"
          >
            <Icon icon={icons.close} className="text-inverse-on-surface" />
          </IconButton>
        ) : null}
      </div>
      <nav
        aria-label={t('nav.drawer.aria')}
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-3 pt-3 pb-3"
      >
        {navGroupsFor(role).map((group) => (
          <div key={group.id} className="flex flex-col">
            <p className="typescale-label-medium px-4 pb-2">{t(group.labelKey)}</p>
            {group.items.map((item) => {
              const isActive = item.href === current;
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  onPress={onNavigate}
                  aria-current={isActive ? 'page' : undefined}
                  className={cx(
                    'typescale-label-large flex h-12 items-center gap-3 rounded-full px-4 no-underline outline-none',

                    isActive ? 'bg-inverse-on-surface/12' : 'hover:bg-inverse-on-surface/8',
                    'focus-visible:outline-solid focus-visible:outline-3 focus-visible:outline-offset-2 focus-visible:outline-inverse-primary',
                  )}
                >
                  <Icon
                    icon={icons[item.icon]}
                    className={isActive ? 'text-inverse-primary' : undefined}
                  />
                  {t(item.labelKey)}
                </Link>
              );
            })}
          </div>
        ))}
      </nav>
      {user ? (
        <div className="mx-3 flex shrink-0 flex-col border-t border-inverse-on-surface/12 pt-3 pb-4">
          {/* Identitas jadi tombol menu akun: satu-satunya jalan ke "Keluar". */}
          <MenuTrigger>
            <RacButton
              aria-label={t('account.menu.menu', { nama_pengguna: user.username })}
              className="flex w-full cursor-pointer flex-col gap-1 rounded-medium px-4 py-3 text-left outline-none hover:bg-inverse-on-surface/8 focus-visible:outline-solid focus-visible:outline-3 focus-visible:outline-offset-2 focus-visible:outline-inverse-primary"
            >
              <span className="typescale-body-medium truncate">{user.username}</span>
              <span className="typescale-label-medium">{enumLabel(i18n, 'peran', user.peran)}</span>
            </RacButton>
            <Menu onAction={() => onKeluar?.()}>
              <MenuItem id="keluar">{t('account.menu.sign_out')}</MenuItem>
            </Menu>
          </MenuTrigger>
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
 * Application shell (desain Penpot `F0 / Shell-*`, `.takeover/design/shell-v2-spec.md`): link
 * lompat, drawer navigasi, bar atas 64 px berisi breadcrumb dan label lingkungan, lalu konten.
 *
 * - large (>= 1200 px): `NavigationDrawer variant="standard"`, selalu tampil, tinggi penuh; bar atas
 *   dan konten ada di kanannya.
 * - di bawahnya (1024, 768, 375): `variant="modal"` dengan scrim, dibuka dari tombol menu di bar atas.
 */
export function AppShell({ pathname = '/', user, role = null, onKeluar, children }: AppShellProps) {
  const { t } = useTranslation();
  const isLarge = useIsLarge();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const closeDrawer = useCallback(() => setDrawerOpen(false), []);
  const [record, setRecord] = useState<CrumbRecord | null>(null);

  // Drawer modal hanya ada di bawah large; diturunkan dari `isLarge` supaya tidak perlu efek
  // sinkronisasi dan tidak pernah tersisa terbuka setelah jendela diperlebar.
  const isModalDrawerOpen = drawerOpen && !isLarge;

  return (
    <div className="flex min-h-dvh bg-surface text-on-surface">
      <a
        href="#main-content"
        className="typescale-label-large sr-only rounded-full bg-primary px-4 py-2 text-on-primary focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50"
      >
        {t('nav.skip_link')}
      </a>

      <NavigationDrawer
        variant={isLarge ? 'standard' : 'modal'}
        aria-label={t('nav.drawer.aria')}
        isOpen={isModalDrawerOpen}
        onOpenChange={(open) => setDrawerOpen(open)}
      >
        <DrawerContent
          pathname={pathname}
          role={role}
          user={user}
          onKeluar={onKeluar}
          onNavigate={isLarge ? undefined : closeDrawer}
          onClose={isLarge ? undefined : closeDrawer}
        />
      </NavigationDrawer>

      <div className="flex min-w-0 flex-1 flex-col">
        <TopAppBar
          className="border-b border-outline-variant"
          leading={
            isLarge ? undefined : (
              <IconButton
                aria-label={t('nav.drawer.open_menu')}
                onPress={() => setDrawerOpen(true)}
                className="-ml-2"
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
            <span className="typescale-label-small rounded-small bg-surface-container-high px-3 py-1 text-on-surface">
              {t('nav.env_label')}
            </span>
          }
        />

        <main id="main-content" tabIndex={-1} className="min-w-0 flex-1 px-6 pt-8 pb-6">
          {/* Konten dibatasi 1200 px (spec bagian 4) supaya tabel panjang tetap terbaca di layar lebar. */}
          <div className="mx-auto flex w-full max-w-[1200px] flex-col gap-3">
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
