import { lazy, Suspense, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { RoleProvider, useRole, type AdminRole } from '../features/common/RoleContext';
import { LogPageScreen } from '../features/log/LogPageScreen';

/*
 * Screen routing for the F0 console (ADR-0003 5.1). F0 is a handful of screens with no nested
 * navigation, so a table lookup on `location.pathname` is the whole router; there is no router
 * dependency in `package.json` and the shell already receives the pathname it renders.
 *
 * SHAA-202/203 own these routes (see `src/app/nav.ts`); the heavier screens are lazy so the shell
 * stays the only thing in the first chunk.
 */
const NikSearchScreen = lazy(() =>
  import('../features/auditor/NikSearchScreen').then((m) => ({ default: m.NikSearchScreen })),
);
const AuditScreen = lazy(() =>
  import('../features/audit-akses/AuditScreen').then((m) => ({ default: m.AuditScreen })),
);
const QuotaScreen = lazy(() =>
  import('../features/quota/QuotaScreen').then((m) => ({ default: m.QuotaScreen })),
);

function NotFound(): ReactNode {
  const { t } = useTranslation();
  return (
    <section className="flex flex-col gap-2">
      {/* The catalog has no dedicated 404 line yet; the 403 wording is the closest existing one. */}
      <h1 className="typescale-headline-small">{t('error.http.403.title')}</h1>
      <p className="typescale-body-medium">{t('error.http.403.body')}</p>
    </section>
  );
}

function Forbidden(): ReactNode {
  const { t } = useTranslation();
  return (
    <section className="flex flex-col gap-2">
      <h1 className="typescale-headline-small">{t('error.http.403.title')}</h1>
      <p className="typescale-body-medium">{t('error.http.403.body')}</p>
    </section>
  );
}

/** `/log/<kode_transaksi>` — S11 opened on one transaction (section 7). */
const LOG_DETAIL = /^\/log\/([^/]+)$/;

/** Path → screen. Longest exact match wins; anything else renders the not-found panel. */
const ROUTE_ROLES: Record<string, readonly AdminRole[]> = {
  '/': ['platform_admin', 'operator', 'auditor', 'client_owner'],
  '/nik': ['auditor', 'platform_admin'],
  '/audit': ['auditor', 'platform_admin'],
  '/kuota': ['platform_admin', 'operator'],
  '/provider': ['platform_admin'],
  '/secret': ['platform_admin'],
  '/upstream': ['platform_admin'],
  '/downstream': ['platform_admin'],
  '/client': ['platform_admin', 'client_owner'],
  '/settings': ['platform_admin', 'operator', 'auditor', 'client_owner'],
};

function allowedFor(pathname: string, role: AdminRole | null): boolean {
  if (role === null) return true;
  const base = pathname.split('/')[1] ? `/${pathname.split('/')[1]}` : '/';
  const allowed = ROUTE_ROLES[base];
  if (!allowed) return true;
  return allowed.includes(role);
}

export function ScreenForPath({ pathname }: { pathname: string }): ReactNode {
  const role = useRole();
  const clean = pathname.replace(/\/+$/, '') || '/';
  if (!allowedFor(clean, role)) return <Forbidden />;
  const logDetail = LOG_DETAIL.exec(clean);
  if (logDetail) {
    return <LogPageScreen initialCode={decodeURIComponent(logDetail[1] ?? '')} />;
  }
  switch (clean) {
    case '/':
      return <LogPageScreen />;
    case '/nik':
      return <NikSearchScreen />;
    case '/audit':
      return <AuditScreen />;
    case '/kuota':
      return <QuotaScreen />;
    case '/provider':
    case '/secret':
    case '/upstream':
    case '/downstream':
    case '/client':
    case '/settings':
      return <NotFound />;
    default:
      if (
        clean.startsWith('/provider/') ||
        clean.startsWith('/upstream/') ||
        clean.startsWith('/client/') ||
        clean.startsWith('/downstream/')
      ) {
        return <NotFound />;
      }
      return <NotFound />;
  }
}

/**
 * Screens sit under the role context so a shared layout does not re-read the session per screen.
 * `null` = no role-specific affordances until the session hook lands (handoff spec section 18.1).
 *
 * Until the session hook lands the app cannot assume a role by itself, so the e2e suite needs a
 * way to pin one. That seam is a **build flag**, not a query parameter:
 * `import.meta.env.VITE_TEST_ROLE_SEAM` is replaced at build time by `vite.config.ts` (true only for
 * the `playwright` and `test` modes) and is `false` in every production build, where the whole
 * branch — and therefore the `?peran=` read itself — is dropped. Design review (SHAA-203) requires that a production URL cannot set the role; the API
 * stays the last guard for every role-gated action either way (section 18.1).
 */
const ROLES: readonly AdminRole[] = ['platform_admin', 'operator', 'auditor', 'client_owner'];

function roleFromSearch(search: string): AdminRole | null {
  if (!import.meta.env.VITE_TEST_ROLE_SEAM) return null;
  const value = new URLSearchParams(search).get('peran');
  return ROLES.find((role) => role === value) ?? null;
}

export function RoutedScreens({
  pathname,
  search = window.location.search,
}: {
  pathname: string;
  search?: string;
}): ReactNode {
  return (
    <RoleProvider current={roleFromSearch(search)}>
      <Suspense fallback={null}>
        <ScreenForPath pathname={pathname} />
      </Suspense>
    </RoleProvider>
  );
}
