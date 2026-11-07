import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';

export interface MfaRecoveryScreenProps {
  codes: string[];
  isOpen: boolean;
  onAcknowledge: () => void;
}

/**
 * S17c — kode pemulihan satu kali. Parent owns `isOpen` and drops `codes` after acknowledge,
 * so values never leak (AC-31). Dialog non-dismissible: only "Sudah saya simpan" exits (S17c).
 */
export function MfaRecoveryScreen({
  codes,
  isOpen,
  onAcknowledge,
}: MfaRecoveryScreenProps): ReactNode {
  const { t } = useTranslation();
  const snackbar = useSnackbar();

  const copyAll = async () => {
    try {
      await navigator.clipboard.writeText(codes.join('\n'));
      snackbar.notify(t('auth.mfa.recovery.copied_all'));
    } catch {
      snackbar.notify(t('auth.mfa.recovery.copy_failed'));
    }
  };

  return (
    <>
      <SnackbarRegion queue={snackbar.queue} />
      <Dialog
        isOpen={isOpen}
        isDismissable={false}
        role="alertdialog"
        title={t('auth.mfa.recovery.title')}
        description={t('auth.mfa.recovery.subtitle')}
        closeLabel={t('common.close')}
        footer={
          <>
            <Button variant="filled-tonal" onPress={() => void copyAll()}>
              {t('auth.mfa.recovery.copy_all')}
            </Button>
            <Button variant="filled" onPress={onAcknowledge}>
              {t('auth.mfa.recovery.acknowledge')}
            </Button>
          </>
        }
      >
        <ol
          aria-label={t('auth.mfa.recovery.list_aria', { count: codes.length })}
          className="typescale-body-medium grid grid-cols-2 gap-2 font-mono"
        >
          {codes.map((code, index) => (
            <li key={code}>
              <span className="text-on-surface-variant">{index + 1}.</span> {code}
            </li>
          ))}
        </ol>
      </Dialog>
    </>
  );
}

export default MfaRecoveryScreen;
