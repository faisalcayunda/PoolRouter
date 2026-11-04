import { useState, type FormEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { api, ApiError, type ApiClient } from '../../api/client';
import { TextField } from '../../components/m3/TextField';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';

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

  const masuk = async (event: FormEvent) => {
    event.preventDefault();
    if (mengirim) return;
    setMengirim(true);
    setGalat(null);
    try {
      await client.login({ username: username.trim(), password });
      // Full navigation: the session cookie is now set and every cached screen must start clean.
      if (onBerhasil) onBerhasil();
      else window.location.assign('/');
    } catch (error) {
      if (error instanceof ApiError) {
        if (error.status === 401) setGalat('invalid');
        else if (error.status === 429 || error.kode === 'login_dibatasi') setGalat('throttled');
        else setGalat('network');
      } else {
        setGalat('network');
      }
    } finally {
      setMengirim(false);
    }
  };

  return (
    <main
      id="konten"
      tabIndex={-1}
      className="flex min-h-screen flex-col items-center justify-center gap-6 bg-surface px-4 py-12"
    >
      <h1 className="typescale-headline-medium text-on-surface">{t('login.page.title')}</h1>
      <p className="typescale-body-large max-w-[36ch] text-center text-on-surface-variant">
        {t('login.page.subtitle')}
      </p>

      {search.includes('sesi_berakhir') ? (
        <div className="w-full max-w-[360px]">
          <InlineAlert tone="info">{t('login.page.session_expired')}</InlineAlert>
        </div>
      ) : null}

      {galat ? (
        <div className="w-full max-w-[360px]">
          <InlineAlert tone="error" live="assertive">
            {galat === 'network' ? t('error.network.load') : t(`login.error.${galat}`)}
          </InlineAlert>
        </div>
      ) : null}

      <form onSubmit={(e) => void masuk(e)} className="flex w-full max-w-[360px] flex-col gap-4">
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
        <Button type="submit" isDisabled={mengirim} className="self-start">
          {t('login.page.submit')}
        </Button>
      </form>
    </main>
  );
}
