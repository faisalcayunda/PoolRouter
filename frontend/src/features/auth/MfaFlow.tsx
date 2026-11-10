import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { ApiError, type ApiClient, type LangkahLogin } from '../../api/client';
import { MfaCodeScreen } from './MfaCodeScreen';
import { MfaQrScreen } from './MfaQrScreen';
import { MfaRecoveryScreen } from './MfaRecoveryScreen';
import { tandaiLoginPemulihan } from './RecoveryBanner';
import { MfaRegistrationCodeScreen } from './MfaRegistrationCodeScreen';

type Tahap = 'kode' | 'daftar' | 'qr' | 'pemulihan';
type Galat = 'invalid' | 'throttled' | 'network';

interface Props {
  client: ApiClient;
  /** Jawaban `POST /auth/login` yang memicu alur ini. */
  langkah: LangkahLogin;
  username: string;
  /** Sesi sudah terbit: pemanggil melakukan navigasi penuh. */
  onSelesai: () => void;
  /** Kembali ke form kata sandi. */
  onKembali: () => void;
}

/**
 * Menyambung S17-S17c ke endpoint MFA (AC-29-31). Rahasia pendaftaran dan kode pemulihan hanya
 * hidup di state komponen ini: tidak masuk cache Query, storage, URL, atau pesan galat.
 */
export function MfaFlow({ client, langkah, username, onSelesai, onKembali }: Props): ReactNode {
  const { t } = useTranslation();
  const [tahap, setTahap] = useState<Tahap>(langkah === 'mfa' ? 'kode' : 'daftar');
  const [mengirim, setMengirim] = useState(false);
  const [galat, setGalat] = useState<Galat | null>(null);
  const [otpauthUri, setOtpauthUri] = useState('');
  const [kunciSetup, setKunciSetup] = useState('');
  const [kodePemulihan, setKodePemulihan] = useState<string[]>([]);

  const pesan = (jenis: 'kode' | 'daftar'): string | null => {
    if (!galat) return null;
    if (galat === 'invalid') {
      return t(
        jenis === 'daftar' ? 'auth.mfa.error.invalid_registration' : 'auth.mfa.error.invalid',
      );
    }
    return t(`auth.mfa.error.${galat}`);
  };

  /** Satu pintu untuk setiap panggilan: kunci ganda kirim, petakan galat, jangan bocorkan isinya. */
  const jalankan = async (kerja: () => Promise<void>) => {
    if (mengirim) return;
    setMengirim(true);
    setGalat(null);
    try {
      await kerja();
    } catch (error) {
      if (error instanceof ApiError) {
        if (error.status === 401 || error.status === 422) setGalat('invalid');
        else if (error.status === 429) setGalat('throttled');
        else setGalat('network');
      } else {
        setGalat('network');
      }
    } finally {
      setMengirim(false);
    }
  };

  const kirimKode = (kode: string) =>
    jalankan(async () => {
      // Panjang 6 = TOTP; selain itu kode pemulihan (MfaCodeScreen sudah menormalkan).
      const totp = /^\d{6}$/.test(kode);
      await client.verifikasiMfa(totp ? { kode } : { kode_pemulihan: kode });
      // AC-32: login lewat kode pemulihan memicu banner info satu kali di shell.
      if (!totp) tandaiLoginPemulihan();
      onSelesai();
    });

  const kirimKodeDaftar = (kodePendaftaran: string) =>
    jalankan(async () => {
      const hasil = await client.daftarMfa({ kode_pendaftaran: kodePendaftaran });
      setOtpauthUri(hasil.otpauth_uri);
      setKunciSetup(hasil.rahasia_base32);
      setTahap('qr');
    });

  const konfirmasi = (kodeTotp: string) =>
    jalankan(async () => {
      const hasil = await client.konfirmasiMfa({ kode_totp: kodeTotp });
      setKodePemulihan(hasil.kode_pemulihan);
      setOtpauthUri('');
      setKunciSetup('');
      setTahap('pemulihan');
    });

  const akui = () => {
    setKodePemulihan([]);
    onSelesai();
  };

  return (
    <main id="main-content" tabIndex={-1} className="outline-none">
      {tahap === 'kode' ? (
        <MfaCodeScreen
          username={username}
          loading={mengirim}
          error={pesan('kode')}
          onSubmit={kirimKode}
          onBack={onKembali}
        />
      ) : null}
      {tahap === 'daftar' ? (
        <MfaRegistrationCodeScreen
          loading={mengirim}
          error={pesan('daftar')}
          onSubmit={kirimKodeDaftar}
          onBack={onKembali}
        />
      ) : null}
      {tahap === 'qr' ? (
        <MfaQrScreen
          otpauthUri={otpauthUri}
          setupKey={kunciSetup}
          account={username}
          loading={mengirim}
          error={pesan('kode')}
          onConfirm={konfirmasi}
        />
      ) : null}
      {tahap === 'pemulihan' ? (
        <MfaRecoveryScreen codes={kodePemulihan} isOpen onAcknowledge={akui} />
      ) : null}
    </main>
  );
}
