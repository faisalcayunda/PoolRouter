import { useRef, useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { HasilK20, PratinjauK20 } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { TextField } from '../../components/m3/TextField';
import { useFormatter } from '../../i18n/useFormatter';
import { errorMessage, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';

/** Batas `AlasanK20.alasan` di kontrak (minLength 10, maxLength 500). */
export const ALASAN_K20_MIN = 10;
export const ALASAN_K20_MAKS = 500;

export interface NonaktifkanDialogProps {
  /** Judul dialog (kalimat katalog milik pemanggil, mis. "Nonaktifkan upstream pool?"). */
  title: string;
  /** Nama objek yang ditampilkan di badan dialog. */
  objekNama: string;
  /** GET `pratinjau-nonaktif`; dibaca sekali begitu dialog dibuka. */
  loadPreview: (signal: AbortSignal) => Promise<PratinjauK20>;
  /** POST `nonaktifkan` dengan alasan yang sudah dipangkas. */
  onConfirm: (alasan: string) => Promise<HasilK20>;
  onClose: () => void;
  /** Dipanggil setelah server menjawab sukses; pemanggil menutup dialog dan memuat ulang daftar. */
  onDone: (hasil: HasilK20) => void;
}

/**
 * S14 dialog nonaktifkan K-20 (section 3a): pratinjau dampak dulu, baru alasan dan konfirmasi.
 * Dialog tidak tahu jenis objeknya; upstream pool, downstream pool, dan (nanti) provider atau
 * endpoint memakai bentuk yang sama. Konfirmasi baru aktif sesudah pratinjau terbaca, supaya
 * admin selalu melihat dampak sebelum menekan tombol. Alasan tidak diingat antar pembukaan.
 */
export function NonaktifkanDialog({
  title,
  objekNama,
  loadPreview,
  onConfirm,
  onClose,
  onDone,
}: NonaktifkanDialogProps): ReactNode {
  const { t, i18n } = useTranslation();
  const fmt = useFormatter();
  const preview = useQuery<PratinjauK20>(loadPreview, 'k20-preview');
  const [alasan, setAlasan] = useState('');
  const [attempted, setAttempted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const pressing = useRef(false);

  const trimmed = alasan.trim();
  const invalid = trimmed.length < ALASAN_K20_MIN || trimmed.length > ALASAN_K20_MAKS;
  const ready = preview.status === 'success';

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    setAttempted(true);
    if (!ready || invalid || saving || pressing.current) return;
    pressing.current = true;
    setSaving(true);
    setFailure(null);
    try {
      onDone(await onConfirm(trimmed));
    } catch (error) {
      setFailure(error);
      setSaving(false);
      pressing.current = false;
    }
  }

  const data = preview.status === 'success' ? preview.data : null;
  const terakhir = data?.upstream_aktif_terakhir_untuk ?? [];

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
          <Button isDisabled={saving || !ready} onPress={() => void submit()}>
            {saving ? t('k20.submitting') : t('k20.confirm')}
          </Button>
        </>
      }
    >
      <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-4 pt-2">
        <p>{t('k20.body', { nama: objekNama })}</p>

        {preview.status === 'loading' ? (
          <p role="status" aria-live="polite">
            {t('common.state.loading')}
          </p>
        ) : null}
        {preview.status === 'error' ? (
          <QueryError error={preview.error} onRetry={preview.reload} />
        ) : null}

        {data ? (
          <section aria-labelledby="k20-dampak" className="flex flex-col gap-2">
            <h3 id="k20-dampak" className="typescale-title-small text-on-surface">
              {t('k20.preview.heading')}
            </h3>
            <ul className="list-disc pl-5">
              <li>{t('k20.preview.cancelled', { jumlah: data.akan_cancelled })}</li>
              <li>{t('k20.preview.failed', { jumlah: data.akan_failed })}</li>
              {typeof data.dipakai_oleh_downstream_pool_count === 'number' ? (
                <li>
                  {t('k20.preview.downstream_count', {
                    jumlah: data.dipakai_oleh_downstream_pool_count,
                  })}
                </li>
              ) : null}
            </ul>
            <p className="typescale-body-small">
              {t('k20.preview.computed', { waktu: fmt.tanggalWaktu(data.dihitung_pada) })}
            </p>
            {terakhir.length > 0 ? (
              <InlineAlert tone="warning" heading={t('k20.preview.last_active_heading')}>
                <ul className="list-disc pl-5">
                  {terakhir.map((pool) => (
                    <li key={pool.downstream_pool_id}>{pool.nama}</li>
                  ))}
                </ul>
              </InlineAlert>
            ) : null}
          </section>
        ) : null}

        {failure !== null ? (
          <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
        ) : null}

        <TextField
          label={t('k20.reason.label')}
          description={t('k20.reason.help', { min: ALASAN_K20_MIN, maks: ALASAN_K20_MAKS })}
          value={alasan}
          onChange={setAlasan}
          autoComplete="off"
          isInvalid={attempted && invalid}
          errorMessage={
            attempted && invalid
              ? t('k20.reason.error', { min: ALASAN_K20_MIN, maks: ALASAN_K20_MAKS })
              : undefined
          }
        />
      </form>
    </Dialog>
  );
}
