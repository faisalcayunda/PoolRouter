import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';
import type { SesiState } from './useSesi';

/** Penanda sesi-browser: login barusan memakai kode pemulihan (AC-32), dipasang oleh `MfaFlow`. */
export const KUNCI_LOGIN_PEMULIHAN = 'pr_recovery_login';

/** Sisa kode pada atau di bawah angka ini memunculkan peringatan di setiap muat shell. */
const AMBANG_SISA = 3;

export function tandaiLoginPemulihan() {
  try {
    sessionStorage.setItem(KUNCI_LOGIN_PEMULIHAN, '1');
  } catch {
    /* storage tidak tersedia: banner info saja yang hilang */
  }
}

function bacaPenanda(): boolean {
  try {
    return sessionStorage.getItem(KUNCI_LOGIN_PEMULIHAN) === '1';
  } catch {
    return false;
  }
}

export interface RecoveryBannerProps {
  sesi: Pick<SesiState, 'me' | 'memuat' | 'galat'>;
}

/**
 * Banner sisa kode pemulihan MFA di shell (AC-32), datanya `GET /auth/me` `mfa.sisa_kode_pemulihan`.
 * - sisa 0: error, tanpa tombol tutup;
 * - sisa 1-3: peringatan di setiap muat shell, "Tutup" hanya menyembunyikan di sesi ini;
 * - sisa > 3: info satu kali setelah login pakai kode pemulihan;
 * - memuat, galat, atau peran tanpa kewajiban MFA: tidak tampil.
 */
export function RecoveryBanner({ sesi }: RecoveryBannerProps) {
  const { t } = useTranslation();
  const [ditutup, setDitutup] = useState(false);
  const [infoTampil] = useState(bacaPenanda);

  const mfa = sesi.me?.mfa;
  const aktif = !sesi.memuat && !sesi.galat && mfa?.wajib === true;

  // Info hanya sekali: penanda dihapus begitu shell sempat memuat data MFA.
  useEffect(() => {
    if (aktif && infoTampil) sessionStorage.removeItem(KUNCI_LOGIN_PEMULIHAN);
  }, [aktif, infoTampil]);

  if (!aktif || !mfa) return null;
  const sisa = mfa.sisa_kode_pemulihan;

  if (sisa > AMBANG_SISA) {
    return infoTampil ? (
      <InlineAlert tone="info" live="polite">
        {t('recovery.info')}
      </InlineAlert>
    ) : null;
  }
  if (sisa === 0) {
    return (
      <InlineAlert tone="error" live="assertive">
        {t('recovery.error')}
      </InlineAlert>
    );
  }
  if (ditutup) return null;
  return (
    <InlineAlert
      tone="warning"
      live="polite"
      action={
        <Button variant="text" onPress={() => setDitutup(true)}>
          {t('common.action.close')}
        </Button>
      }
    >
      {t('recovery.warning', { n: sisa })}
    </InlineAlert>
  );
}
