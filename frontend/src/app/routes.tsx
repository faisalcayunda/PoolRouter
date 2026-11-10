import { lazy, Suspense, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { RoleProvider, type AdminRole } from '../features/common/RoleContext';
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
const ProviderListScreen = lazy(() =>
  import('../features/provider/ProviderListScreen').then((m) => ({
    default: m.ProviderListScreen,
  })),
);
const ProviderFormScreen = lazy(() =>
  import('../features/provider/ProviderFormScreen').then((m) => ({
    default: m.ProviderFormScreen,
  })),
);
const SecretScreen = lazy(() =>
  import('../features/secret/SecretScreen').then((m) => ({ default: m.SecretScreen })),
);

const ClientListScreen = lazy(() =>
  import('../features/client/ClientListScreen').then((m) => ({ default: m.ClientListScreen })),
);
const ClientDetailScreen = lazy(() =>
  import('../features/client/ClientDetailScreen').then((m) => ({
    default: m.ClientDetailScreen,
  })),
);
const CredentialsScreen = lazy(() =>
  import('../features/client/CredentialsScreen').then((m) => ({ default: m.CredentialsScreen })),
);
const WebhookScreen = lazy(() =>
  import('../features/client/WebhookScreen').then((m) => ({ default: m.WebhookScreen })),
);
const UpstreamListScreen = lazy(() =>
  import('../features/pool/UpstreamListScreen').then((m) => ({ default: m.UpstreamListScreen })),
);
const UpstreamFormScreen = lazy(() =>
  import('../features/pool/UpstreamFormScreen').then((m) => ({ default: m.UpstreamFormScreen })),
);
const DownstreamListScreen = lazy(() =>
  import('../features/pool/DownstreamListScreen').then((m) => ({
    default: m.DownstreamListScreen,
  })),
);
const DownstreamFormScreen = lazy(() =>
  import('../features/pool/DownstreamFormScreen').then((m) => ({
    default: m.DownstreamFormScreen,
  })),
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

/** `/log/<kode_transaksi>` — S11 opened on one transaction (section 7). */
const LOG_DETAIL = /^\/log\/([^/]+)$/;

/** `/provider/<id>` — form sunting provider (S03–S04, ARC-101); `/provider/baru` membuat baru. */
const PROVIDER_DETAIL = /^\/provider\/([^/]+)$/;

/** `/client/<id>[/kredensial|/webhook]` — S08, S08b, S09 dan S09a (ARC-104). */
const CLIENT_DETAIL = /^\/client\/([^/]+)(?:\/(kredensial|webhook))?$/;
/** `/upstream/<id>` dan `/downstream/<id>` — form pool (S06b, S07, ARC-102); `baru` membuat baru. */
const UPSTREAM_DETAIL = /^\/upstream\/([^/]+)$/;
const DOWNSTREAM_DETAIL = /^\/downstream\/([^/]+)$/;

/** Path → screen. Longest exact match wins; anything else renders the not-found panel. */
export function ScreenForPath({ pathname }: { pathname: string }): ReactNode {
  const clean = pathname.replace(/\/+$/, '') || '/';
  const logDetail = LOG_DETAIL.exec(clean);
  if (logDetail) {
    // The code is never a NIK (section 18.3), so it is safe to keep in the URL.
    return <LogPageScreen initialCode={decodeURIComponent(logDetail[1] ?? '')} />;
  }
  const providerDetail = PROVIDER_DETAIL.exec(clean);
  if (providerDetail) {
    const id = decodeURIComponent(providerDetail[1] ?? '');
    return <ProviderFormScreen providerId={id === 'baru' ? undefined : id} />;
  }
  const clientDetail = CLIENT_DETAIL.exec(clean);
  if (clientDetail) {
    const id = decodeURIComponent(clientDetail[1] ?? '');
    if (clientDetail[2] === 'kredensial') return <CredentialsScreen clientId={id} />;
    if (clientDetail[2] === 'webhook') return <WebhookScreen clientId={id} />;
    return <ClientDetailScreen clientId={id} />;
  }
  const upstreamDetail = UPSTREAM_DETAIL.exec(clean);
  if (upstreamDetail) {
    const id = decodeURIComponent(upstreamDetail[1] ?? '');
    return <UpstreamFormScreen poolId={id === 'baru' ? undefined : id} />;
  }
  const downstreamDetail = DOWNSTREAM_DETAIL.exec(clean);
  if (downstreamDetail) {
    const id = decodeURIComponent(downstreamDetail[1] ?? '');
    return <DownstreamFormScreen poolId={id === 'baru' ? undefined : id} />;
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
      return <ProviderListScreen />;
    case '/secret':
      return <SecretScreen />;
    case '/client':
      return <ClientListScreen />;
    case '/upstream':
      return <UpstreamListScreen />;
    case '/downstream':
      return <DownstreamListScreen />;
    default:
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
  role,
}: {
  pathname: string;
  search?: string;
  /** Session role; the test seam still wins so Playwright can pin a role. */
  role?: AdminRole | null;
}): ReactNode {
  return (
    <RoleProvider current={roleFromSearch(search) ?? role ?? null}>
      <Suspense fallback={null}>
        <ScreenForPath pathname={pathname} />
      </Suspense>
    </RoleProvider>
  );
}
