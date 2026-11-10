import { useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { TextField } from '../../components/m3/TextField';
import { Icon, icons } from '../../icons';

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
  /** Client ID kredensial baru; hanya diisi layar kredensial (layar webhook tidak punya padanannya). */
  clientId?: string;
  onDone: () => void;
}): ReactNode {
  const { t } = useTranslation();
  const secretId = useId();
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
      title={
        <span className="flex flex-col items-start gap-4">
          <span className="flex size-10 items-center justify-center rounded-full bg-surface-container-low text-success">
            <Icon icon={icons.checkCircle} />
          </span>
          {title}
        </span>
      }
      closeLabel={t('common.close')}
      footer={<Button onPress={onDone}>{t('secret_once.dialog.done')}</Button>}
    >
      <div className="flex flex-col gap-4 pt-2">
        <p>{t('secret_once.dialog.body')}</p>
        {clientId !== undefined ? (
          <TextField
            label={t('client.col.id')}
            value={clientId}
            isReadOnly
            autoComplete="off"
            className="[&_input]:font-code"
          />
        ) : null}
        <div className="flex flex-col gap-1.5">
          <label
            htmlFor={secretId}
            className="typescale-body-medium font-medium text-on-surface-variant"
          >
            {fieldLabel}
          </label>
          <div className="flex h-10 items-center justify-between gap-2 rounded-medium border border-outline-variant bg-surface-container-lowest pl-3 pr-1">
            <input
              id={secretId}
              readOnly
              autoComplete="off"
              value={value}
              aria-describedby={`${secretId}-once`}
              className="m3-focus typescale-body-medium min-w-0 flex-1 bg-transparent font-code text-on-surface"
            />
            <Button variant="outlined" onPress={() => void copy()}>
              {t('common.action.copy')}
            </Button>
          </div>
          <p id={`${secretId}-once`} className="typescale-body-small text-on-surface-variant">
            {t('secret_once.dialog.shown_once')}
          </p>
        </div>
        <p role="status" className="typescale-body-medium text-on-surface-variant">
          {copied ? t('secret_once.dialog.copied') : ''}
        </p>
      </div>
    </Dialog>
  );
}
