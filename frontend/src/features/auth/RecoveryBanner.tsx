import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Button } from '../../components/m3/Button';
import type { Me } from '../../api/client';
import type { SesiState } from './useSesi';

export interface RecoveryBannerProps {
  sesi?: SesiState;
  me?: Me | null;
  memuat?: boolean;
  galat?: boolean;
}

const KUNCI_INFO = 'pr_recovery_login';

function bacaInfoFlag(): boolean {
  try {
    return sessionStorage.getItem(KUNCI_INFO) === '1';
  } catch {
    return false;
  }
}

function hapusInfoFlag() {
  try {
    sessionStorage.removeItem(KUNCI_INFO);
  } catch {
    /* ignore */
  }
}

export function RecoveryBanner({ sesi, me: meProp, memuat: memuatProp, galat: galatProp }: RecoveryBannerProps) {
  const { t } = useTranslation();
  const me = sesi ? sesi.me : (meProp ?? null);
  const memuat = sesi ? sesi.memuat : (memuatProp ?? false);
  const galat = sesi ? sesi.galat : (galatProp ?? false);

  const mfa = me?.mfa;
  const wajib = mfa?.wajib === true;

  const [ditutup, setDitutup] = useState(false);
  const [infoTampil] = useState(() => bacaInfoFlag());

  useEffect(() => {
    if (infoTampil && mfa && mfa.sisa_kode_pemulihan > 3 && wajib) {
      hapusInfoFlag();
    }
  }, [infoTampil, mfa, wajib]);

  if (memuat || galat || !me || !mfa || !wajib) return null;

  const sisa = mfa.sisa_kode_pemulihan;

  if (sisa > 3) {
    if (!infoTampil) return null;
    return (
      <div className="mx-auto w-full max-w-[1200px] px-4 pt-4">
        <InlineAlert tone="info" live="polite">
          {t('recovery.info')}
        </InlineAlert>
      </div>
    );
  }

  if (ditutup) return null;

  if (sisa === 0) {
    return (
      <div className="mx-auto w-full max-w-[1200px] px-4 pt-4">
        <InlineAlert tone="error" live="assertive">
          {t('recovery.error')}
        </InlineAlert>
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-[1200px] px-4 pt-4">
      <InlineAlert tone="warning" live="polite">
        {t('recovery.warning', { n: sisa })}
      </InlineAlert>
      <div className="pt-2">
        <Button variant="text" onPress={() => setDitutup(true)}>
          {t('common.action.close')}
        </Button>
      </div>
    </div>
  );
}
