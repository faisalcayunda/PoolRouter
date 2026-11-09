import { Suspense, useCallback, useEffect, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-aria-components';
import { Breadcrumbs } from '../components/m3/Breadcrumbs';
import { cx } from '../components/m3/cx';
import { IconButton } from '../components/m3/IconButton';
import { NavigationDrawer } from '../components/m3/NavigationDrawer';
import { TopAppBar } from '../components/m3/TopAppBar';
import { enumLabel } from '../i18n/enums';
import { Icon, icons } from '../icons';
import type { AdminRole } from '../features/common/RoleContext';
import { activeHref, crumbsFor, navGroupsFor } from './nav';

/** M3 window size class breakpoint for the `expanded` layout (tailwind.css `--breakpoint-expanded`). */
const EXPANDED_QUERY = '(min-width: 840px)';

function useIsExpanded(): boolean {
  const [expanded, setExpanded] = useState(
    () => typeof window !== 'undefined' && window.matchMedia(EXPANDED_QUERY).matches,
  );
  useEffect(() => {
    const mql = window.matchMedia(EXPANDED_QUERY);
    const onChange = (e: MediaQueryListEvent) => setExpanded(e.matches);
    mql.addEventListener('change', onChange);
    return () => mql.removeEventListener('change', onChange);
  }, []);
  return expanded;
}

function DrawerNav({
  pathname,
  role,
  onNavigate,
}: {
  pathname: string;
  role: AdminRole | null;
  onNavigate?: () => void;
}) {
  const { t } = useTranslation();
  const current = activeHref(pathname);
  return (
    <nav aria-label={t('nav.drawer.aria')} className="flex flex-col gap-3">
      {navGroupsFor(role).map((group) => (
        <div key={group.id} className="flex flex-col gap-0.5">
          <p className="typescale-label-small px-3 text-inverse-on-surface/70">{t(group.labelKey)}</p>
          {group.items.map((item) => {
            const isActive = item.href === current;
            return (
              <Link
                key={item.href}
                href={item.href}
                onPress={onNavigate}
                aria-current={isActive ? 'page' : undefined}
                className={cx(
                  'typescale-label-large rounded-full px-3 py-2.5 no-underline outline-none',
                  isActive
                    ? 'bg-inverse-primary text-inverse-surface'
                    : 'text-inverse-on-surface hover:bg-inverse-on-surface/8',
                  'focus-visible:outline-2 focus-visible:outline-inverse-primary',
                )}
              >
                {t(item.labelKey)}
              </Link>
            );
          })}
        </div>
      ))}
    </nav>
  );
}

export interface AppShellProps {
  /** Current route path, used to mark the active drawer item. */
  pathname?: string;
  /** Signed-in identity shown at the right of the top app bar (spec section 13.1). */
  user?: { username: string; peran: string } | null;
  /** Admin role that filters the drawer; `null` = unknown, full menu (API is the last guard). */
  role?: AdminRole | null;
  /** Screen rendered inside `<main>`. */
  children?: ReactNode;
}

/**
 * Application shell: skip link, top app bar, navigation drawer and content area.
 *
 * Window size classes (ADR-0003 5.1):
 * - compact / medium (< 840 px): the drawer is `NavigationDrawer variant="modal"`,
 *   opened from the menu button.
 * - expanded and up (>= 840 px): `NavigationDrawer variant="standard"`, always visible.
 */
export function AppShell({ pathname = '/', user, role = null, children }: AppShellProps) {
  const { t, i18n } = useTranslation();
  const isExpanded = useIsExpanded();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const closeDrawer = useCallback(() => setDrawerOpen(false), []);

  // The modal drawer only exists in compact/medium; deriving it from `isExpanded`
  // avoids a state-sync effect and can never show a stale open drawer after resizing.
  const isModalDrawerOpen = drawerOpen && !isExpanded;

  return (
    <div className="min-h-dvh bg-surface text-on-surface">
      <a
        href="#main-content"
        className="typescale-label-large sr-only rounded-full bg-primary px-4 py-2 text-on-primary focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50"
      >
        {t('nav.skip_link')}
      </a>

      <TopAppBar
        className="h-16 border-b border-on-surface/12 px-2"
        leading={
          isExpanded ? undefined : (
            <IconButton aria-label={t('nav.drawer.open_menu')} onClick={() => setDrawerOpen(true)}>
              <Icon icon={icons.menu} />
            </IconButton>
          )
        }
        trailing={
          user ? (
            <span className="typescale-body-medium text-on-surface-variant">
              {user.username} · {enumLabel(i18n, 'peran', user.peran)}
            </span>
          ) : undefined
        }
        title={<span className="font-brand px-2 text-primary">{t('common.app.name')}</span>}
      />

      <div className="flex">
        <NavigationDrawer
          variant={isExpanded ? 'standard' : 'modal'}
          aria-label={t('nav.drawer.aria')}
          isOpen={isModalDrawerOpen}
          onOpenChange={(open) => setDrawerOpen(open)}
        >
          <DrawerNav pathname={pathname} role={role} onNavigate={isExpanded ? undefined : closeDrawer} />
        </NavigationDrawer>

        <main id="main-content" tabIndex={-1} className="min-w-0 flex-1 p-4">
          {/* Section 4: content is capped at 1200 px so long tables stay readable on a wide screen. */}
          <div className="mx-auto flex w-full max-w-[1200px] flex-col gap-3">
            <Breadcrumbs
              aria-label={t('nav.breadcrumb.aria')}
              items={crumbsFor(pathname).map((c) => ({
                label: c.labelKey ? t(c.labelKey) : c.text,
                ...(c.href ? { href: c.href } : {}),
              }))}
            />
            <Suspense
              fallback={<p className="typescale-body-medium">{t('common.state.loading')}</p>}
            >
              {children}
            </Suspense>
          </div>
        </main>
      </div>
    </div>
  );
}
