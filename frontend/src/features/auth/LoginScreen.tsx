import { useState, type FormEvent } from 'react';
import { Trans, useTranslation } from 'react-i18next';
import { api, ApiError, type ApiClient } from '../../api/client';
import { TextField } from '../../components/m3/TextField';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';

type Step = 'login' | 'mfa' | 'daftar' | 'konfirmasi' | 'pemulihan';

interface Props {
  client?: ApiClient;
  search?: string;
  onBerhasil?: () => void;
}

export function LoginScreen({ client = api, search = '', onBerhasil }: Props) {
  const { t } = useTranslation();
  const [step, setStep] = useState<Step>('login');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [kode, setKode] = useState('');
  const [kodeDaftar, setKodeDaftar] = useState('');
  const [kodeTotp, setKodeTotp] = useState('');
  const [secret, setSecret] = useState('');
  const [recoveryCodes, setRecoveryCodes] = useState<string[]>([]);
  const [modePemulihan, setModePemulihan] = useState(false);
  const [mengirim, setMengirim] = useState(false);
  const [galat, setGalat] = useState<string | null>(null);
  const [tungguDetik, setTungguDetik] = useState<number | null>(null);
  const [sisaKode, setSisaKode] = useState<number | null>(null);

  const selesai = () => {
    if (onBerhasil) onBerhasil();
    else window.location.assign('/');
  };

  const masuk = async (e: FormEvent) => {
    e.preventDefault();
    if (mengirim) return;
    setMengirim(true);
    setGalat(null);
    try {
      const res = await client.login({ username: username.trim(), password });
      if (!res) {
        selesai();
        return;
      }
      if (res.langkah === 'mfa') setStep('mfa');
      else if (res.langkah === 'daftar_mfa') setStep('daftar');
      else selesai();
    } catch (err) {
      if (err instanceof ApiError) {
        if (err.status === 401) setGalat('invalid');
        else if (err.status === 429 || err.kode === 'login_dibatasi') {
          setGalat('throttled');
          setTungguDetik(err.retryAfterDetik);
        } else setGalat('network');
      } else setGalat('network');
    } finally {
      setMengirim(false);
    }
  };

  const verifikasi = async (e: FormEvent) => {
    e.preventDefault();
    if (mengirim) return;
    setMengirim(true);
    setGalat(null);
    try {
      await client.mfa(
        modePemulihan ? { kode_pemulihan: kode.replace(/\s+/g, '').toUpperCase() } : { kode: kode.replace(/\s+/g, '') },
      );
      try {
        const me = await client.me();
        const sisa = me.mfa?.sisa_kode_pemulihan;
        if (typeof sisa === 'number') setSisaKode(sisa);
      } catch {
        /* ignore */
      }
      selesai();
    } catch (err) {
      setGalat(err instanceof ApiError ? (err.status === 401 ? 'invalid' : 'network') : 'network');
    } finally {
      setMengirim(false);
    }
  };

  const mulaiDaftar = async (e: FormEvent) => {
    e.preventDefault();
    if (mengirim) return;
    setMengirim(true);
    setGalat(null);
    try {
      const res = await client.mfaDaftar({ kode_pendaftaran: kodeDaftar.trim() });
      setSecret(res.rahasia_base32);
      setStep('konfirmasi');
    } catch (err) {
      setGalat(err instanceof ApiError ? 'invalid' : 'network');
    } finally {
      setMengirim(false);
    }
  };

  const konfirmasi = async (e: FormEvent) => {
    e.preventDefault();
    if (mengirim) return;
    setMengirim(true);
    setGalat(null);
    try {
      const res = await client.mfaKonfirmasi({ kode_totp: kodeTotp.replace(/\s+/g, '') });
      setRecoveryCodes(res.kode_pemulihan ?? []);
      setStep('pemulihan');
    } catch (err) {
      setGalat(err instanceof ApiError ? 'invalid' : 'network');
    } finally {
      setMengirim(false);
    }
  };

  const selesaiDialog = () => {
    setSecret('');
    setRecoveryCodes([]);
    selesai();
  };

  const card = (children: React.ReactNode) => (
    <main id="main-content" tabIndex={-1} className="flex min-h-screen flex-col items-center justify-center gap-6 bg-surface px-4 py-12">
      <div className="w-full max-w-[428px] rounded-2xl border border-outline-variant bg-surface-container-lowest p-8 flex flex-col gap-4">
        {children}
      </div>
    </main>
  );

  if (step === 'login') {
    return (
      <main id="main-content" tabIndex={-1} className="flex min-h-screen flex-col items-center justify-center gap-6 bg-surface px-4 py-12">
        <h1 className="typescale-headline-medium text-on-surface">{t('login.page.title')}</h1>
        {search.includes('sesi_berakhir') ? (
          <div className="w-full max-w-[360px]"><InlineAlert tone="info">{t('login.page.session_expired')}</InlineAlert></div>
        ) : null}
        {galat ? (
          <div className="w-full max-w-[360px]">
            <InlineAlert tone="error" live="assertive">
              {galat === 'network' ? t('login.error.network') : galat === 'throttled' && tungguDetik !== null ? (
                tungguDetik >= 60 ? (
                  <Trans i18nKey="login.error.throttled_menit" values={{ menit: Math.ceil(tungguDetik / 60) }} components={{ b: <span className="font-semibold tabular-nums" /> }} />
                ) : (
                  <Trans i18nKey="login.error.throttled_retry" values={{ detik: tungguDetik }} components={{ b: <span className="font-semibold tabular-nums" /> }} />
                )
              ) : t(`login.error.${galat}`)}
            </InlineAlert>
          </div>
        ) : null}
        <form onSubmit={(e) => void masuk(e)} className="flex w-full max-w-[360px] flex-col gap-4">
          <TextField label={t('login.page.username')} value={username} isDisabled={mengirim} onChange={setUsername} autoComplete="username" isRequired />
          <TextField label={t('login.page.password')} type="password" value={password} isDisabled={mengirim} onChange={setPassword} autoComplete="current-password" isRequired />
          <Button type="submit" isDisabled={mengirim}>{t('login.page.submit')}</Button>
        </form>
      </main>
    );
  }

  if (step === 'mfa') {
    return card(
      <>
        <h1 className="typescale-title-large text-on-surface">Verifikasi dua langkah</h1>
        <p className="typescale-body-medium text-on-surface-variant">Halo, {username}</p>
        {galat ? <InlineAlert tone="error" live="assertive">{galat === 'invalid' ? 'Kode tidak valid' : 'Gangguan jaringan'}</InlineAlert> : null}
        <form onSubmit={(e) => void verifikasi(e)} className="flex flex-col gap-4">
          {modePemulihan ? (
            <TextField
              label="Kode pemulihan"
              value={kode}
              isDisabled={mengirim}
              onChange={setKode}
              inputMode="text"
              autoComplete="off"
              maxLength={12}
              isRequired
            />
          ) : (
            <TextField
              label="Kode authenticator"
              value={kode}
              isDisabled={mengirim}
              onChange={(v) => setKode(v.replace(/\s/g, ''))}
              inputMode="numeric"
              autoComplete="one-time-code"
              maxLength={6}
              isRequired
            />
          )}
          <Button type="submit" isDisabled={mengirim}>{mengirim ? 'Memverifikasi…' : 'Verifikasi'}</Button>
        </form>
        <button type="button" className="text-primary text-sm" onClick={() => setModePemulihan((m) => !m)}>
          {modePemulihan ? 'Pakai kode authenticator' : 'Pakai kode pemulihan'}
        </button>
        <button type="button" className="text-primary text-sm" onClick={() => setStep('login')}>Kembali ke login</button>
        {sisaKode !== null && sisaKode <= 3 ? (
          <InlineAlert tone="warning">Sisa {sisaKode} kode pemulihan. Minta reset ke platform admin.</InlineAlert>
        ) : null}
      </>,
    );
  }

  if (step === 'daftar') {
    return card(
      <>
        <h1 className="typescale-title-large text-on-surface">Daftarkan authenticator</h1>
        <p className="typescale-body-medium text-on-surface-variant">Langkah 1 dari 3</p>
        {galat ? <InlineAlert tone="error" live="assertive">Kode pendaftaran tidak valid</InlineAlert> : null}
        <form onSubmit={(e) => void mulaiDaftar(e)} className="flex flex-col gap-4">
          <TextField label="Kode pendaftaran" value={kodeDaftar} isDisabled={mengirim} onChange={(v) => setKodeDaftar(v.toUpperCase().replace(/\s|-/g, ''))} inputMode="text" autoComplete="off" isRequired />
          <Button type="submit" isDisabled={mengirim}>{mengirim ? 'Memverifikasi…' : 'Lanjut'}</Button>
        </form>
        <button type="button" className="text-primary text-sm" onClick={() => setStep('login')}>Kembali ke login</button>
      </>,
    );
  }

  if (step === 'konfirmasi') {
    return card(
      <>
        <h1 className="typescale-title-large text-on-surface">Pindai kode QR</h1>
        <p className="typescale-body-medium text-on-surface-variant">Langkah 2 dari 3</p>
        <div role="img" aria-label="Kode QR pendaftaran PoolRouter untuk aplikasi authenticator. Bila tidak bisa memindai, pakai kunci setup di bawahnya." className="flex items-center justify-center w-[200px] h-[200px] border border-dashed border-outline mx-auto">
          <span className="text-on-surface-variant text-xs">QR</span>
        </div>
        <div className="font-mono text-sm break-all bg-surface-container-lowest border border-outline p-2 rounded">{secret.match(/.{1,4}/g)?.join(' ')}</div>
        <button type="button" className="text-primary text-sm" onClick={() => navigator.clipboard?.writeText(secret)}>Salin</button>
        {galat ? <InlineAlert tone="error" live="assertive">Kode tidak valid</InlineAlert> : null}
        <form onSubmit={(e) => void konfirmasi(e)} className="flex flex-col gap-4">
          <TextField label="Kode dari authenticator" value={kodeTotp} isDisabled={mengirim} onChange={(v) => setKodeTotp(v.replace(/\s/g, ''))} inputMode="numeric" autoComplete="one-time-code" maxLength={6} isRequired />
          <Button type="submit" isDisabled={mengirim}>{mengirim ? 'Memverifikasi…' : 'Konfirmasi'}</Button>
        </form>
      </>,
    );
  }

  // pemulihan dialog
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-inverse-surface/40">
      <div className="w-full max-w-[480px] rounded-2xl bg-surface-container-high p-6 flex flex-col gap-4">
        <h2 className="typescale-title-large text-on-surface">Simpan kode pemulihan</h2>
        <ol className="grid grid-cols-2 gap-2 font-mono text-sm">
          {recoveryCodes.map((c, i) => (
            <li key={i}>{i + 1}. {c}</li>
          ))}
        </ol>
        <div className="flex gap-2">
          <Button variant="outlined" onPress={() => navigator.clipboard?.writeText(recoveryCodes.join('\n'))}>Salin semua</Button>
          <Button onPress={selesaiDialog}>Sudah saya simpan</Button>
        </div>
      </div>
    </div>
  );
}
