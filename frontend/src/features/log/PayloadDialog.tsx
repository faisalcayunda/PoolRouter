import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, ApiError, type ApiClient, type PayloadView } from '../../api/client';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Button } from '../../components/m3/Button';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useFormatter } from '../../i18n/useFormatter';
import { labelOr } from '../../i18n/labelOr';
import { accessTone } from './accessTone';
import { ReasonDialog } from '../common/ReasonDialog';
import { LoadingState } from '../common/page';
import { ScrollArea } from '../common/ScrollArea';

/**
 * Section 18.3 payload structure: a summary plus three read-only JSON blocks. Values that are
 * absent render as "Tidak tersedia" rather than an empty cell, and there is no copy/download
 * action anywhere in this dialog: the payload lives only while the dialog is open.
 */
type PayloadBlock = 'input' | 'metadata' | 'hasil';

const BLOCKS: PayloadBlock[] = ['input', 'metadata', 'hasil'];

function JsonBlock({
  label,
  value,
  emptyLabel,
}: {
  label: string;
  value: unknown;
  emptyLabel: string;
}) {
  const empty =
    value === null ||
    value === undefined ||
    (typeof value === 'object' && Object.keys(value).length === 0);
  return (
    <section className="flex flex-col gap-1">
      <h3 className="typescale-title-small">{label}</h3>
      {empty ? (
        <p className="typescale-body-medium text-on-surface-variant">{emptyLabel}</p>
      ) : (
        <ScrollArea label={label} className="max-h-64 overflow-auto">
          <pre className="typescale-body-small m3-focus rounded-small bg-surface-container p-3 text-on-surface">
            {JSON.stringify(value, null, 2)}
          </pre>
        </ScrollArea>
      )}
    </section>
  );
}

export interface PayloadDialogProps {
  isOpen: boolean;
  /** Transaction code the payload belongs to; also used as the dialog heading. */
  kodeTransaksi: string;
  /** Reason carried over from the NIK search; the user may edit it before opening. */
  initialReason: string;
  client?: ApiClient;
  onClose: () => void;
  /** Fired once a payload has actually been fetched, so the caller can re-run the audit list. */
  onOpened?: () => void;
}

/**
 * S12b / payload dialog. Opening always asks for a reason first (a new one per opening: the
 * server writes a `lihat_payload` audit row each time), then renders the read-only view. A 410
 * `payload_dihapus` is a normal outcome after retention, not an error to retry.
 */
export function PayloadDialog({
  isOpen,
  kodeTransaksi,
  initialReason,
  client = api,
  onClose,
  onOpened,
}: PayloadDialogProps): ReactNode {
  const { t, i18n: instance } = useTranslation();
  const format = useFormatter();
  const [phase, setPhase] = useState<'reason' | 'loading' | 'open' | 'error'>('reason');
  // Holds the reason of a *failed* attempt so a retry does not make the auditor retype it; the
  // first opening always starts from the caller's reason (section 18.3).
  const [reasonDraft, setReasonDraft] = useState<string | null>(null);
  const [payload, setPayload] = useState<PayloadView | null>(null);
  const [errorKode, setErrorKode] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  function reset() {
    setPhase('reason');
    setPayload(null);
    setErrorKode(null);
    setReasonDraft(null);
  }

  async function open(nextReason: string) {
    setSubmitting(true);
    setPhase('loading');
    // Remember the reason only to pre-fill a retry; it is cleared by `reset()` on close, so the
    // next opening starts from a fresh reason (section 18.3).
    setReasonDraft(nextReason);
    try {
      const body = await client.bukaPayload(
        { kode_transaksi: kodeTransaksi, alasan: nextReason },
        { alasan: nextReason },
      );
      setPayload(body);
      setPhase('open');
      onOpened?.();
    } catch (error) {
      setErrorKode(error instanceof ApiError ? error.kode : 'jaringan');
      setPhase('error');
    } finally {
      setSubmitting(false);
    }
  }

  const purged = errorKode === 'payload_dihapus' || errorKode === 'retensi_habis';

  return (
    <>
      <ReasonDialog
        isOpen={isOpen && phase === 'reason'}
        title={t('nik.payload.title')}
        body={t('nik.payload.body')}
        initialReason={reasonDraft ?? initialReason}
        confirmLabel={t('nik.payload.open')}
        isSubmitting={submitting}
        onConfirm={(next) => void open(next)}
        onCancel={() => {
          reset();
          onClose();
        }}
      />

      <Dialog
        isOpen={isOpen && phase !== 'reason'}
        onOpenChange={(open) => {
          if (!open) {
            // Closing drops the payload from the screen; nothing is cached (section 18.3).
            reset();
            onClose();
          }
        }}
        title={t('nik.payload.title', { kode: kodeTransaksi })}
        closeLabel={t('common.action.close')}
        footer={
          <Button
            variant="text"
            onPress={() => {
              reset();
              onClose();
            }}
          >
            {t('common.action.close')}
          </Button>
        }
      >
        {phase === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}

        {phase === 'error' ? (
          <InlineAlert tone="error" live="polite">
            {purged
              ? t('nik.page.retention') // GAP: log.payload.error_purged
              : errorKode === 'jaringan'
                ? t('error.network.load')
                : t('risk.common.action_failed')}
          </InlineAlert>
        ) : null}

        {phase === 'open' && payload ? (
          <div className="flex flex-col gap-4">
            <p className="typescale-body-small text-on-surface-variant">{t('nik.payload.body')}</p>
            <section className="flex flex-col gap-1">
              {/* GAP: log.payload.summary — no catalog line yet; `log.page.title` names the block. */}
              <h3 className="typescale-title-small">{t('log.page.title')}</h3>
              {/* The summary is composed from the identifying fields, not a free-form block. */}
              <dl className="typescale-body-medium grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
                <dt className="text-on-surface-variant">{t('log.col.code')}</dt>
                <dd>{payload.kode_transaksi}</dd>
                <dt className="text-on-surface-variant">{t('log.col.time')}</dt>
                <dd>{format.tanggalWaktu(payload.requested_at)}</dd>
                <dt className="text-on-surface-variant">{t('log.col.access')}</dt>
                <dd>
                  <StatusLabel tone={accessTone(payload.status_akses)}>
                    {labelOr(
                      instance,
                      `enum.status_akses.${payload.status_akses}`,
                      payload.status_akses,
                    )}
                  </StatusLabel>
                </dd>
              </dl>
            </section>
            {BLOCKS.map((block) => (
              <JsonBlock
                key={block}
                // GAP: log.payload.{input,metadata,hasil} — the whole block is absent from the
                // v0.13 catalog. The data-contract field name is the honest fallback.
                label={block}
                value={payload[block]}
                // GAP: log.payload.not_available — an em dash carries "no value" without inventing
                // a sentence; replace with the catalog line once Fitcher adds it.
                emptyLabel="—"
              />
            ))}
          </div>
        ) : null}
      </Dialog>
    </>
  );
}
