import { useEffect, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { PratinjauK20 } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { TextField } from '../../components/m3/TextField';
import { errorMessage } from '../common/page';
import { REASON_MAX, REASON_MIN } from '../common/ReasonDialog';

export interface K20DialogProps {
  title: string;
  confirmLabel: string;
  /** Teks cakupan dan efek (katalog `risk.*`), dirender di atas pratinjau. */
  scope: ReactNode;
  /** Catatan di bawah angka; `reversible` hanya untuk client yang bisa diaktifkan lagi. */
  notes: ReactNode;
  loadPreview: (signal: AbortSignal) => Promise<PratinjauK20>;
  run: (alasan: string) => Promise<unknown>;
  onClose: () => void;
  onDone: (result: unknown) => void;
}

/**
 * Dialog K-20 untuk nonaktifkan client dan cabut kredensial: pratinjau dampak dihitung server
 * sebelum konfirmasi, alasan 10 sampai 500 karakter wajib, dan angka final ada di hasil aksi.
 * Tombol konfirmasi mati sampai pratinjau berhasil dimuat, supaya tidak ada konfirmasi buta.
 */
export function K20Dialog({
  title,
  confirmLabel,
  scope,
  notes,
  loadPreview,
  run,
  onClose,
  onDone,
}: K20DialogProps): ReactNode {
  const { t, i18n } = useTranslation();
  const [preview, setPreview] = useState<PratinjauK20 | null>(null);
  const [previewError, setPreviewError] = useState<unknown>(null);
  const [nonce, setNonce] = useState(0);
  const [alasan, setAlasan] = useState('');
  const [attempted, setAttempted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);

  useEffect(() => {
    const controller = new AbortController();
    loadPreview(controller.signal).then(
      (data) => {
        if (!controller.signal.aborted) {
          setPreview(data);
          setPreviewError(null);
        }
      },
      (error: unknown) => {
        if (!controller.signal.aborted) setPreviewError(error);
      },
    );
    return () => controller.abort();
    // `loadPreview` is created per dialog opening; `nonce` is the retry handle.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nonce]);

  const trimmed = alasan.trim();
  const alasanError =
    trimmed.length < REASON_MIN || trimmed.length > REASON_MAX
      ? t('log.payload.reason.error')
      : null;

  async function confirm() {
    setAttempted(true);
    if (busy || preview === null || alasanError) return;
    setBusy(true);
    setFailure(null);
    try {
      onDone(await run(trimmed));
    } catch (error) {
      setFailure(error);
      setBusy(false);
    }
  }

  const nothing = preview !== null && preview.akan_cancelled === 0 && preview.akan_failed === 0;

  return (
    <Dialog
      isOpen
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={title}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onClose}>
            {t('common.action.cancel')}
          </Button>
          <Button destructive isDisabled={busy || preview === null} onPress={() => void confirm()}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 pt-2">
        {scope}
        <section aria-label={t('risk.common.preview_heading')} className="flex flex-col gap-2">
          <h3 className="typescale-title-small">{t('risk.common.preview_heading')}</h3>
          {preview === null && previewError === null ? (
            <p role="status">{t('clients.k20.preview_loading')}</p>
          ) : null}
          {previewError !== null ? (
            <InlineAlert
              tone="error"
              action={
                <Button
                  variant="text"
                  onPress={() => {
                    setPreviewError(null);
                    setNonce((n) => n + 1);
                  }}
                >
                  {t('common.action.retry')}
                </Button>
              }
            >
              {errorMessage(t, i18n, previewError).message}
            </InlineAlert>
          ) : null}
          {preview !== null ? (
            <ul className="list-disc pl-5">
              {nothing ? <li>{t('risk.common.body_zero')}</li> : null}
              {preview.akan_cancelled > 0 ? (
                <li>{t('risk.common.cancelled', { count: preview.akan_cancelled })}</li>
              ) : null}
              {preview.akan_failed > 0 ? (
                <li>{t('risk.common.failed', { count: preview.akan_failed })}</li>
              ) : null}
            </ul>
          ) : null}
          <p className="typescale-body-small text-on-surface-variant">
            {t('risk.common.estimate_note')}
          </p>
        </section>
        {notes}
        {failure !== null ? (
          <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
        ) : null}
        <TextField
          label={t('clients.k20.reason')}
          description={t('clients.k20.reason_help')}
          value={alasan}
          onChange={setAlasan}
          autoComplete="off"
          isRequired
          isInvalid={attempted && alasanError !== null}
          errorMessage={alasanError}
        />
      </div>
    </Dialog>
  );
}
