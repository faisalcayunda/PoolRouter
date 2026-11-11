import { startTransition, useCallback, useEffect, useState } from 'react';
import { RouterProvider } from 'react-aria-components';
import { useTranslation } from 'react-i18next';
import { LoginScreen } from '../features/auth/LoginScreen';
import { RecoveryBanner } from '../features/auth/RecoveryBanner';
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
        <RecoveryBanner sesi={sesi} />
        <RoutedScreens pathname={pathname} search={search} role={peranShell(sesi.me.peran)} />
      </AppShell>
      <SesiDialogs sesi={sesi} />
    </>
  );
}

/** Layar yang memang butuh dokumen baru (sesi sendiri ada di sini); semua path lain dirender sisi klien. */
const DI_LUAR_SPA = '/login';

interface Lokasi {
  pathname: string;
  search: string;
}

/**
 * Router sisi klien paling sederhana: state pathname + `history.pushState` + `popstate`, diteruskan
 * ke `Link` react-aria lewat `RouterProvider`. Pembaruan dibungkus `startTransition` supaya layar
 * lama tetap tampil sampai chunk lazy layar baru siap (tanpa shell kosong atau kedip). `Link`
 * eksternal, `target=_blank`, dan klik dengan modifier tidak dicegat oleh react-aria.
 */
function useLokasi(awal: Lokasi) {
  const [lokasi, setLokasi] = useState<Lokasi>(awal);

  useEffect(() => {
    const onPopState = () =>
      startTransition(() =>
        setLokasi({ pathname: window.location.pathname, search: window.location.search }),
      );
    window.addEventListener('popstate', onPopState);
    return () => window.removeEventListener('popstate', onPopState);
  }, []);

  const navigate = useCallback((href: string) => {
    const tujuan = new URL(href, window.location.href);
    if (tujuan.pathname === DI_LUAR_SPA) {
      window.location.assign(href);
      return;
    }
    window.history.pushState(null, '', tujuan.pathname + tujuan.search + tujuan.hash);
    startTransition(() => setLokasi({ pathname: tujuan.pathname, search: tujuan.search }));
  }, []);

  return { lokasi, navigate };
}

/**
 * App root: `/login` is the only out-of-shell screen, everything else needs a session. A 401 in
 * this branch is announced by the API client, and `useSesi` navigates to
 * `/login?alasan=sesi_berakhir` (spec section 7). `pathname` dan `search` hanya lokasi awal;
 * navigasi berikutnya dipegang `useLokasi`, jadi shell dan sesi tidak ter-mount ulang.
 */
export function AppRoot({ pathname = '/', search = '' }: { pathname?: string; search?: string }) {
  const { lokasi, navigate } = useLokasi({ pathname, search });
  return (
    <RouterProvider navigate={navigate}>
      {lokasi.pathname === DI_LUAR_SPA ? (
        <LoginScreen search={lokasi.search} />
      ) : (
        <SignedIn pathname={lokasi.pathname} search={lokasi.search} />
      )}
    </RouterProvider>
  );
}
