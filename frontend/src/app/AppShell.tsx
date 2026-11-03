import { Suspense, useCallback, useEffect, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-aria-components';
import { cx } from '../components/m3/cx';
import { IconButton } from '../components/m3/IconButton';
import { Icon, icons } from '../icons';
import { activeHref, NAV_GROUPS } from './nav';

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

function DrawerNav({ pathname, onNavigate }: { pathname: string; onNavigate?: () => void }) {
  const { t } = useTranslation();
  const current = activeHref(pathname);
  return (
    <nav aria-label={t('nav.drawer.aria')} className="flex flex-col gap-3">
      {NAV_GROUPS.map((group) => (
        <div key={group.id} className="flex flex-col gap-0.5">
          <p className="typescale-label-small px-3 text-on-surface-variant">{t(group.labelKey)}</p>
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
                    ? 'bg-secondary-container text-on-secondary-container'
                    : 'text-on-surface hover:bg-on-surface/8 focus-visible:outline-2 focus-visible:outline-primary',
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
  /** Screen rendered inside `<main>`. */
  children?: ReactNode;
}

/**
 * Application shell: skip link, top app bar, navigation drawer and content area.
 *
 * Window size classes (ADR-0003 5.1):
 * - compact / medium (< 840 px): the drawer is modal, opened from the menu button.
 * - expanded and up (>= 840 px): the drawer is permanently visible.
 */
export function AppShell({ pathname = '/', children }: AppShellProps) {
  const { t } = useTranslation();
  const isExpanded = useIsExpanded();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const closeDrawer = useCallback(() => setDrawerOpen(false), []);

  // The modal drawer only exists in compact/medium; deriving it from `isExpanded`
  // avoids a state-sync effect and can never show a stale open drawer after resizing.
  const isModalDrawerOpen = drawerOpen && !isExpanded;

  useEffect(() => {
    if (!isModalDrawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') closeDrawer();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [isModalDrawerOpen, closeDrawer]);

  return (
    <div className="min-h-dvh bg-surface text-on-surface">
      <a
        href="#main-content"
        className="typescale-label-large sr-only rounded-full bg-primary px-4 py-2 text-on-primary focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50"
      >
        {t('nav.skip_link')}
      </a>

      <header className="flex h-16 items-center gap-2 border-b border-on-surface/12 px-2">
        {!isExpanded && (
          <IconButton
            aria-label={t('nav.drawer.open_menu')}
            onClick={() => setDrawerOpen(true)}
          >
            <Icon icon={icons.menu} />
          </IconButton>
        )}
        <span className="typescale-title-large font-brand px-2 text-primary">
          {t('common.app.name')}
        </span>
      </header>

      <div className="flex">
        {isExpanded ? (
          <aside className="w-72 shrink-0 border-r border-on-surface/12 p-3">
            <DrawerNav pathname={pathname} />
          </aside>
        ) : (
          isModalDrawerOpen && (
            <div className="fixed inset-0 z-40 flex">
              <button
                type="button"
                aria-label={t('common.action.close')}
                onClick={closeDrawer}
                className="absolute inset-0 bg-on-surface/32"
              />
              <aside
                role="dialog"
                aria-modal="true"
                aria-label={t('nav.drawer.aria')}
                className="relative z-10 w-72 bg-surface-container p-3 shadow-level1"
              >
                <DrawerNav pathname={pathname} onNavigate={closeDrawer} />
              </aside>
            </div>
          )
        )}

        <main id="main-content" tabIndex={-1} className="min-w-0 flex-1 p-4">
          <Suspense fallback={<p className="typescale-body-medium">{t('common.state.loading')}</p>}>
            {children}
          </Suspense>
        </main>
      </div>
    </div>
  );
}
