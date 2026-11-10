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
  clientId,
  onDone,
}: {
  title: string;
  fieldLabel: string;
  value: string;
  /** Client ID kredensial baru; desain v2 menampilkannya di atas secret. Kosong untuk secret webhook. */
  clientId?: string;
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
      footer={<Button onPress={onDone}>{t('secret_once.dialog.done')}</Button>}
    >
      <div className="flex flex-col gap-3 pt-2">
        <p>{t('secret_once.dialog.body')}</p>
        {clientId ? (
          <TextField
            label={t('client.col.id')}
            value={clientId}
            isReadOnly
            autoComplete="off"
            className="[&_input]:font-code"
          />
        ) : null}
        <div className="flex items-start gap-3">
          <TextField
            label={fieldLabel}
            description={t('secret_once.dialog.shown_once')}
            value={value}
            isReadOnly
            autoComplete="off"
            className="min-w-0 flex-1 [&_input]:font-code"
          />
          <Button variant="outlined" className="mt-6" onPress={() => void copy()}>
            {t('common.action.copy')}
          </Button>
        </div>
        <p role="status" className="typescale-body-medium text-on-surface-variant">
          {copied ? t('secret_once.dialog.copied') : ''}
        </p>
      </div>
    </Dialog>
  );
}
