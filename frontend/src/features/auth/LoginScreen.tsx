import { useState, type FormEvent } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { api, ApiError, type ApiClient } from '../../api/client';
import { TextField } from '../../components/m3/TextField';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { Divider } from '../../components/m3/Divider';

type GalatLogin = 'invalid' | 'throttled' | 'network' | null;

interface Props {
  client?: ApiClient;
  search?: string;
  /** Success hook; production navigates, tests assert the call instead of a real reload. */
  onBerhasil?: () => void;
}

/** S01 Masuk: the only screen outside the shell frame (handoff spec section 13.1). */
export function LoginScreen({ client = api, search = '', onBerhasil }: Props) {
  const { t } = useTranslation();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [mengirim, setMengirim] = useState(false);
  const [galat, setGalat] = useState<GalatLogin>(null);
  // Only the server's `Retry-After` can make the throttle copy specific; without it the catalog
  // fallback stands (ADR-0003 3.6: the wait window is the server's, never a guess in the UI).
  const [tungguDetik, setTungguDetik] = useState<number | null>(null);

  const masuk = async (event: FormEvent) => {
    event.preventDefault();
    if (mengirim) return;
    setMengirim(true);
    setGalat(null);
    setTungguDetik(null);
    try {
      await client.login({ username: username.trim(), password });
      // Full navigation: the session cookie is now set and every cached screen must start clean.
      if (onBerhasil) onBerhasil();
      else window.location.assign('/');
    } catch (error) {
      if (error instanceof ApiError) {
        if (error.status === 401) setGalat('invalid');
        else if (error.status === 429 || error.kode === 'login_dibatasi') {
          setGalat('throttled');
          setTungguDetik(error.retryAfterDetik);
        } else setGalat('network');
      } else {
        setGalat('network');
      }
    } finally {
      setMengirim(false);
    }
  };

  return (
    <main
      id="main-content"
      tabIndex={-1}
      className="flex min-h-screen items-center justify-center bg-surface px-4 py-12"
    >
      {/* Penpot S01: one card, 428 px on desktop (padding 36) and the full width minus 16 on mobile (padding 24). */}
      <div className="flex w-full max-w-[428px] flex-col gap-6 rounded-large border border-outline-variant bg-surface-container-lowest p-6 shadow-[0_8px_24px_rgb(16_24_40/0.16)] min-[480px]:p-9">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <span
              aria-hidden="true"
              className="flex h-8 w-8 items-center justify-center rounded-small bg-primary-container"
            >
              <svg
                width="24"
                height="24"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                className="text-on-primary"
                focusable="false"
              >
                <polyline points="4,12 10,12 12,6 14,18 16,12 20,12" />
              </svg>
            </span>
            <span className="typescale-title-large text-on-surface">{t('login.page.brand')}</span>
          </div>
          <StatusLabel>{t('login.page.env')}</StatusLabel>
        </div>
        <Divider />

        <div className="flex flex-col gap-1.5">
          <h1
            tabIndex={-1}
            className="typescale-title-large m3-focus rounded-small text-on-surface outline-none"
          >
            {t('login.page.title')}
          </h1>
          <p className="typescale-body-medium text-on-surface-variant">
            {t('login.page.subtitle')}
          </p>
        </div>

        {search.includes('sesi_berakhir') ? (
          <InlineAlert tone="info">{t('login.page.session_expired')}</InlineAlert>
        ) : null}

        {galat ? (
          <InlineAlert tone="error" live="assertive">
            {galat === 'network' ? (
              t('login.error.network')
            ) : galat === 'throttled' && tungguDetik !== null ? (
              // A wait of a minute or more reads as minutes; listing "251 detik" is unhelpful.
              tungguDetik >= 60 ? (
                <Trans
                  i18nKey="login.error.throttled_menit"
                  values={{ menit: Math.ceil(tungguDetik / 60) }}
                  components={{ b: <span className="font-semibold tabular-nums" /> }}
                />
              ) : (
                <Trans
                  i18nKey="login.error.throttled_retry"
                  values={{ detik: tungguDetik }}
                  components={{ b: <span className="font-semibold tabular-nums" /> }}
                />
              )
            ) : (
              t(`login.error.${galat}`)
            )}
          </InlineAlert>
        ) : null}

        <form onSubmit={(e) => void masuk(e)} className="flex flex-col gap-6">
          <TextField
            label={t('login.page.username')}
            value={username}
            isDisabled={mengirim}
            onChange={setUsername}
            autoComplete="username"
            isRequired
          />
          <TextField
            label={t('login.page.password')}
            type="password"
            value={password}
            isDisabled={mengirim}
            onChange={setPassword}
            autoComplete="current-password"
            isRequired
          />
          <Button type="submit" isDisabled={mengirim} className="w-full">
            {t('login.page.submit')}
          </Button>
        </form>

        <p className="typescale-body-small pt-2 text-on-surface-variant">{t('login.page.help')}</p>
      </div>
    </main>
  );
}
