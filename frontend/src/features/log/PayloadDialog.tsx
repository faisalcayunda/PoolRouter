import { useEffect, useRef, useState, type ReactNode } from 'react';
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
  // Aborts the in-flight `bukaPayload` when the dialog closes or unmounts. A late response for a
  // closed dialog must never land in state: the component stays mounted by its callers, so a
  // payload left behind would show up under the *next* transaction's heading (section 18.3).
  const inFlight = useRef<AbortController | null>(null);
  // Monotonic token; a response is applied only if it still owns the current request.
  const requestId = useRef(0);

  /** Retire whatever is on the wire: the token makes any late response stale, and the abort frees it. */
  function cancelInFlight() {
    requestId.current += 1;
    inFlight.current?.abort();
    inFlight.current = null;
  }

  /** Drop the payload and the reason. Called only from event handlers, never from render. */
  function reset() {
    cancelInFlight();
    setPhase('reason');
    setPayload(null);
    setErrorKode(null);
    setReasonDraft(null);
    setSubmitting(false);
  }

  // Closing the dialog — by the dialog itself, or by the caller flipping `isOpen` — drops
  // everything, so the next opening starts with a fresh reason. The state is adjusted during
  // render (React's "adjust state when a prop changes" pattern) so the stale payload is never
  // painted; only the ref work waits for the effect.
  const [wasOpen, setWasOpen] = useState(isOpen);
  if (isOpen !== wasOpen) {
    setWasOpen(isOpen);
    if (!isOpen) {
      setPhase('reason');
      setPayload(null);
      setErrorKode(null);
      setReasonDraft(null);
      setSubmitting(false);
    }
  }

  // Abort whatever is still on the wire whenever the dialog closes, and on unmount: nothing may
  // resolve into a closed dialog or a gone component. `requestId` is bumped here too, so a
  // response that slipped in between the render and this effect is still dropped, not rendered.
  useEffect(() => {
    if (!isOpen) cancelInFlight();
    return () => cancelInFlight();
  }, [isOpen]);

  async function open(nextReason: string) {
    cancelInFlight();
    const controller = new AbortController();
    inFlight.current = controller;
    const id = requestId.current;
    setSubmitting(true);
    setPhase('loading');
    // Remember the reason only to pre-fill a retry; it is cleared by `reset()` on close, so the
    // next opening starts from a fresh reason (section 18.3).
    setReasonDraft(nextReason);
    try {
      const body = await client.bukaPayload(
        { kode_transaksi: kodeTransaksi, alasan: nextReason },
        { alasan: nextReason, signal: controller.signal },
      );
      if (id !== requestId.current) return; // dialog closed while this was in flight
      setPayload(body);
      setPhase('open');
      onOpened?.();
    } catch (error) {
      if (id !== requestId.current) return; // abandoned request, not a failure to report
      setErrorKode(error instanceof ApiError ? error.kode : 'jaringan');
      setPhase('error');
    } finally {
      if (id === requestId.current) {
        inFlight.current = null;
        setSubmitting(false);
      }
    }
  }

  const purged = errorKode === 'payload_dihapus' || errorKode === 'retensi_habis';

  return (
    <>
      {/* Section 13.2 (S12b): the reason step is titled "Alasan membuka payload", and section 18.3
          uses the same text as the field label. The payload step gets "Payload transaksi". */}
      <ReasonDialog
        isOpen={isOpen && phase === 'reason'}
        title={t('nik.payload.title')}
        body={t('log.payload.reason.supporting')}
        fieldLabel={t('nik.payload.title')}
        initialReason={reasonDraft ?? initialReason}
        confirmLabel={t('log.payload.open')}
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
        title={t('log.payload.dialog_title')}
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
          // A purged payload is a normal outcome ("Tidak tersedia." with the reason), not a
          // retryable error (section 7 state matrix); only real failures get the error alert.
          purged || errorKode === 'tanpa_input' ? (
            <div className="flex flex-col gap-1" role="status" aria-live="polite">
              <p className="typescale-body-large text-on-surface">
                {t('log.payload.not_available')}
              </p>
              <p className="typescale-body-medium text-on-surface-variant">
                {errorKode === 'tanpa_input'
                  ? t('log.payload.rejected_no_input')
                  : t('log.payload.error_purged')}
              </p>
            </div>
          ) : (
            <InlineAlert tone="error" live="polite">
              {errorKode === 'jaringan' ? t('error.network.load') : t('risk.common.action_failed')}
            </InlineAlert>
          )
        ) : null}

        {phase === 'open' && payload ? (
          <div className="flex flex-col gap-4">
            <p className="typescale-body-small text-on-surface-variant">{t('nik.payload.body')}</p>
            <section className="flex flex-col gap-1">
              <h3 className="typescale-title-small">{t('log.payload.summary')}</h3>
              {/* The summary is composed from the identifying fields, not a free-form block. */}
              <dl className="typescale-body-medium grid grid-cols-[auto_1fr] gap-x-4 gap-y-1">
                <dt className="text-on-surface-variant">{t('log.col.code')}</dt>
                <dd>{payload.kode_transaksi}</dd>
                <dt className="text-on-surface-variant">{t('log.col.requested_at')}</dt>
                <dd>{format.tanggalWaktu(payload.requested_at)}</dd>
                <dt className="text-on-surface-variant">{t('log.col.request_id')}</dt>
                <dd>{payload.request_id}</dd>
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
                label={t(`log.payload.${block}`)}
                value={payload[block]}
                emptyLabel={t('log.payload.not_available')}
              />
            ))}
          </div>
        ) : null}
      </Dialog>
    </>
  );
}
