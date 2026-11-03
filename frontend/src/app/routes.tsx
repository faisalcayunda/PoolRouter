import { lazy, Suspense, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { RoleProvider } from '../features/common/RoleContext';
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

/** Path → screen. Longest exact match wins; anything else renders the not-found panel. */
export function ScreenForPath({ pathname }: { pathname: string }): ReactNode {
  const clean = pathname.replace(/\/+$/, '') || '/';
  switch (clean) {
    case '/':
      return <LogPageScreen />;
    case '/nik':
      return <NikSearchScreen />;
    case '/audit':
      return <AuditScreen />;
    case '/kuota':
      return <QuotaScreen />;
    default:
      return <NotFound />;
  }
}

/**
 * Screens sit under the role context so a shared layout does not re-read the session per screen.
 * `null` = no role-specific affordances until the session hook lands (handoff spec section 18.1).
 */
export function RoutedScreens({ pathname }: { pathname: string }): ReactNode {
  return (
    <RoleProvider current={null}>
      <Suspense fallback={null}>
        <ScreenForPath pathname={pathname} />
      </Suspense>
    </RoleProvider>
  );
}
