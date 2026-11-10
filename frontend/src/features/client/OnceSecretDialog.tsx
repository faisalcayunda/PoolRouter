import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { TextField } from '../../components/m3/TextField';

/**
 * Secret yang hanya tampil sekali (kredensial baru, secret webhook). Nilai hidup di props dialog
 * ini saja: pemanggil membuang state-nya saat `onDone`, jadi komponen ter-unmount dan nilainya
 * tidak tertinggal di DOM, cache, URL, atau Snackbar (R-16). Dialog tidak bisa ditutup dengan Esc
 * atau klik di luar; hanya tombol "Sudah saya simpan".
 */
export function OnceSecretDialog({
  title,
  fieldLabel,
  value,
  onDone,
}: {
  title: string;
  fieldLabel: string;
  value: string;
  onDone: () => void;
}): ReactNode {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
    } catch {
      // Clipboard ditolak (konteks tidak aman): pengguna masih bisa memilih teksnya sendiri.
      setCopied(false);
    }
  }

  return (
    <Dialog
      isOpen
      role="alertdialog"
      isDismissable={false}
      isKeyboardDismissDisabled
      title={title}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={() => void copy()}>
            {t('common.action.copy')}
          </Button>
          <Button onPress={onDone}>{t('secret_once.dialog.done')}</Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 pt-2">
        <p>{t('secret_once.dialog.body')}</p>
        <TextField
          label={fieldLabel}
          description={t('secret_once.dialog.shown_once')}
          value={value}
          isReadOnly
          autoComplete="off"
          className="[&_input]:font-code"
        />
        <p role="status" className="typescale-body-medium text-on-surface-variant">
          {copied ? t('secret_once.dialog.copied') : ''}
        </p>
      </div>
    </Dialog>
  );
}
