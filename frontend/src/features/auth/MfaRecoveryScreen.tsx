import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../components/m3/Button';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';

export interface MfaRecoveryScreenProps {
  codes: string[];
  onAcknowledge?: () => void;
  sisaKode?: number;
}

/** S17c — kode pemulihan satu kali, dialog non-dismissible (role alertdialog). */
export function MfaRecoveryScreen({ codes, onAcknowledge }: MfaRecoveryScreenProps): ReactNode {
  const { t } = useTranslation();
  const snackbar = useSnackbar();
  const [open, setOpen] = useState(true);

  const copyAll = async () => {
    try {
      await navigator.clipboard.writeText(codes.join('\n'));
      snackbar.notify(t('auth.mfa.recovery.copied_all'));
    } catch {
      snackbar.notify(t('auth.mfa.recovery.copy_failed'));
    }
  };

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-on-surface/32 p-6">
      <SnackbarRegion queue={snackbar.queue} />
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="mfa-recovery-title"
        className="w-full max-w-[560px] rounded-extra-large bg-surface-container-high p-6 shadow-level3"
      >
        <h2 id="mfa-recovery-title" className="typescale-headline-small">
          {t('auth.mfa.recovery.title')}
        </h2>
        <p className="typescale-body-medium text-on-surface-variant">
          {t('auth.mfa.recovery.subtitle')}
        </p>
        <ol className="grid grid-cols-2 gap-2 font-mono text-sm">
          {codes.map((c, i) => (
            <li key={i}>
              <span className="text-on-surface-variant">{i + 1}.</span> {c}
            </li>
          ))}
        </ol>
        <div className="mt-4 flex justify-end gap-2">
          <Button variant="filled-tonal" onClick={copyAll}>
            {t('auth.mfa.recovery.copy_all')}
          </Button>
          <Button
            variant="filled"
            onClick={() => {
              setOpen(false);
              onAcknowledge?.();
            }}
          >
            {t('auth.mfa.recovery.acknowledge')}
          </Button>
        </div>
      </div>
    </div>
  );
}

export default MfaRecoveryScreen;
