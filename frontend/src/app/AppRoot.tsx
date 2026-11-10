import { useTranslation } from 'react-i18next';
import { LoginScreen } from '../features/auth/LoginScreen';
import { SesiDialogs } from '../features/auth/SesiDialogs';
import { useSesi } from '../features/auth/useSesi';
import type { AdminRole } from '../features/common/RoleContext';
import { AppShell } from './AppShell';
import { RoutedScreens } from './routes';

const ROLES: AdminRole[] = ['platform_admin', 'operator', 'auditor', 'client_owner'];

/** Session identity is a list in the contract (section 8.1); the shell only knows the admin roles. */
export function peranShell(peran: string[] | undefined): AdminRole | null {
  const found = peran?.find((value): value is AdminRole => ROLES.includes(value as AdminRole));
  return found ?? null;
}

/** Authenticated branch: loads the session once and renders the shell with its warnings. */
function SignedIn({ pathname, search }: { pathname: string; search: string }) {
  const { t } = useTranslation();
  const sesi = useSesi({
    onBerakhir: () => window.location.assign('/login?alasan=sesi_berakhir'),
    onKeluar: () => window.location.assign('/login'),
  });

  if (sesi.memuat) {
    return (
      <main
        id="main-content"
        tabIndex={-1}
        className="flex min-h-screen items-center justify-center"
      >
        <p className="typescale-body-large">{t('common.state.loading')}</p>
      </main>
    );
  }

  if (!sesi.me) return <LoginScreen search={search} />;

  return (
    <>
      <AppShell
        pathname={pathname}
        user={{ username: sesi.me.username, peran: sesi.me.peran?.[0] ?? '' }}
        role={peranShell(sesi.me.peran)}
      >
        <RoutedScreens pathname={pathname} search={search} role={peranShell(sesi.me.peran)} />
      </AppShell>
      <SesiDialogs sesi={sesi} />
    </>
  );
}

/**
 * App root: `/login` is the only out-of-shell screen, everything else needs a session. A 401 in
 * this branch is announced by the API client, and `useSesi` navigates to
 * `/login?alasan=sesi_berakhir` (spec section 7).
 */
export function AppRoot({ pathname = '/', search = '' }: { pathname?: string; search?: string }) {
  if (pathname === '/login') return <LoginScreen search={search} />;
  return <SignedIn pathname={pathname} search={search} />;
}
