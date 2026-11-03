import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-aria-components';
import {
  ApiError,
  api,
  type ApiClient,
  type EksporBerkas,
  type LogBaris,
  type LogPage,
  type NikCariRequest,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Icon, icons } from '../../icons';
import { Card } from '../../components/m3/Card';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { TextField } from '../../components/m3/TextField';
import { useFormatter } from '../../i18n/useFormatter';
import type { MessageId } from '../../i18n/types';
import { DateRangeField, type DateRange } from '../common/DateRangeField';
import { ReasonDialog } from '../common/ReasonDialog';
import { PageHeader, EmptyState, LoadingState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { ScrollArea } from '../common/ScrollArea';
import { accessTone } from '../log/accessTone';
import { PayloadDialog } from '../log/PayloadDialog';

const NIK_LENGTH = 16;

function defaultRange(): DateRange {
  const now = new Date();
  const start = new Date(now.getTime() - 24 * 3600 * 1000);
  const iso = (d: Date) => {
    const wib = new Date(d.getTime() + 7 * 3600 * 1000);
    return { date: wib.toISOString().slice(0, 10), time: wib.toISOString().slice(11, 16) };
  };
  const s = iso(start);
  const e = iso(now);
  return {
    startDate: s.date,
    startTime: s.time,
    endDate: e.date,
    endTime: e.time,
    from: `${s.date}T${s.time}:00+07:00`,
    to: `${e.date}T${e.time}:00+07:00`,
  };
}

export interface NikSearchScreenProps {
  client?: ApiClient;
}

/**
 * S12 — Pencarian log per NIK (auditor only). The NIK is mandatory and exactly 16 digits, the
 * reason dialog comes before the request, and every page turn repeats the reason so the audit row
 * matches the page the auditor actually saw (ADR-0003 rev 3 3.6a–c).
 */
export function NikSearchScreen({ client = api }: NikSearchScreenProps): ReactNode {
  const { t } = useTranslation();
  const format = useFormatter();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.nik_search'));

  const [nik, setNik] = useState('');
  const [nikTouched, setNikTouched] = useState(false);
  const [range, setRange] = useState<DateRange | null>(() => defaultRange());
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [reasonOpen, setReasonOpen] = useState(false);
  const [reason, setReason] = useState('');
  const [submitted, setSubmitted] = useState<{ body: NikCariRequest; key: string } | null>(null);
  // Opening a payload is a second audited action, so the dialog lives with this screen.
  const [payloadFor, setPayloadFor] = useState<LogBaris | null>(null);
  // Export (R-18) is its own audited action with its own reason and its own 10k-row ceiling.
  const [exportOpen, setExportOpen] = useState(false);
  const [exportNotice, setExportNotice] = useState<'too_large' | 'error' | null>(null);
  const [exporting, setExporting] = useState(false);

  const nikValid = /^\d{16}$/.test(nik.trim());
  const nikError = nikTouched && !nikValid ? t('nik.page.error') : null;

  const query = useQuery<LogPage>(
    (signal) => client.cariLogNik(submitted!.body, { signal, alasan: submitted!.body.alasan }),
    submitted?.key ?? 'idle',
    { enabled: submitted !== null },
  );

  function requestReason() {
    setNikTouched(true);
    if (!nikValid) return;
    if (!range) {
      // A NIK without a range is the case the catalog already words for this screen.
      setRangeError(t('nik.page.range_only_note'));
      return;
    }
    setRangeError(null);
    setReasonOpen(true);
  }

  function search(nextReason: string) {
    setReason(nextReason);
    setReasonOpen(false);
    const body: NikCariRequest = {
      nik: nik.trim(),
      dari: range!.from,
      sampai: range!.to,
      alasan: nextReason,
    };
    setSubmitted({ body, key: JSON.stringify(body) });
  }

  /**
   * Hand the finished CSV to the browser. Kept inline because the export is the only download in
   * F0; the object URL is revoked on the next task so the browser has already started the
   * download before the blob is released, and nothing is leaked.
   */
  function saveDownload(berkas: EksporBerkas) {
    const url = URL.createObjectURL(berkas.blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = berkas.filename;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }

  async function runExport(alasan: string) {
    setExportOpen(false);
    setExporting(true);
    setExportNotice(null);
    try {
      const berkas = await client.eksporLog(
        { nik: nik.trim(), dari: range!.from, sampai: range!.to, alasan },
        { alasan },
      );
      saveDownload(berkas);
    } catch (error) {
      setExportNotice(
        error instanceof ApiError && error.kode === 'ekspor_terlalu_besar' ? 'too_large' : 'error',
      );
    } finally {
      setExporting(false);
    }
  }

  const rows = query.status === 'success' ? query.data.baris : [];

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={t('nik.page.title')}
        subtitle={t('nik.page.subtitle')}
        titleRef={titleRef}
      />

      <Card variant="outlined">
        <form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            requestReason();
          }}
          aria-describedby="nik-range-note"
        >
          <p id="nik-range-note" className="typescale-body-small text-on-surface-variant">
            {t('nik.page.range_only_note')}
          </p>
          <TextField
            label={t('nik.page.field')}
            description={t('nik.page.field_help')}
            value={nik}
            // `inputmode` keeps numeric keyboards on mobile; the value is still validated server-side.
            inputMode="numeric"
            autoComplete="off"
            isRequired
            isInvalid={Boolean(nikError)}
            errorMessage={nikError ?? undefined}
            onChange={(value) => {
              // Strip formatting *after* paste: `maxLength` would cut "3175 0000 0000 0001" at 16
              // characters before the spaces were removed, silently losing the last digits.
              setNik(value.replace(/\D/g, '').slice(0, NIK_LENGTH));
              setNikTouched(true);
            }}
          />
          <DateRangeField
            legend={t('log.filter.range')}
            description={t('log.filter.range')}
            value={range}
            onChange={(next) => {
              setRange(next);
              if (next) setRangeError(null);
            }}
            errorMessage={rangeError}
          />
          <div>
            <Button variant="filled" type="submit">
              {t('common.action.search')}
            </Button>
          </div>
        </form>
      </Card>

      <p className="typescale-body-small text-on-surface-variant">{t('nik.page.retention')}</p>

      {/* "Ekspor hasil" belongs to S12 (handoff spec section 6): enabled once a NIK is valid. */}
      <div>
        <Button
          variant="outlined"
          isDisabled={!nikValid || exporting}
          onPress={() => {
            setNikTouched(true);
            if (!nikValid || !range) return;
            setExportNotice(null);
            setExportOpen(true);
          }}
        >
          <Icon icon={icons.download} aria-hidden="true" />
          {exporting ? t('common.state.loading') : t('nik.page.export')}
        </Button>
      </div>

      {exportNotice === 'too_large' ? (
        <InlineAlert tone="warning" live="polite" heading={t('nik.page.error')}>
          {t('export.error.too_large')}
        </InlineAlert>
      ) : null}
      {exportNotice === 'error' ? (
        <InlineAlert tone="error" live="assertive" heading={t('risk.common.action_failed')}>
          {t('error.network.load')}
        </InlineAlert>
      ) : null}

      {query.status === 'error' ? <QueryError error={query.error} onRetry={query.reload} /> : null}
      {query.status === 'loading' && submitted ? (
        <LoadingState label={t('common.state.loading')} />
      ) : null}

      {query.status === 'success' ? (
        rows.length === 0 ? (
          <EmptyState title={t('nik.empty.title')} body={t('common.empty.logs.body')} />
        ) : (
          <Card variant="filled" className="p-0">
            <ScrollArea label={t('nik.page.title')}>
              <table className="w-full border-collapse">
                <caption className="sr-only">{t('nik.page.title')}</caption>
                <thead>
                  <tr>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('log.col.time')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('log.col.code')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('log.col.client')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('log.col.nik')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('log.col.access')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('nik.page.payload')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={`${row.kode_transaksi}-${row.requested_at}`} className="h-10">
                      <td className="typescale-body-medium px-4">
                        {format.tanggalWaktu(row.requested_at)}
                      </td>
                      <td className="typescale-body-medium px-4">
                        <Link
                          href={`/log/${encodeURIComponent(row.kode_transaksi)}`}
                          className="m3-focus rounded-small text-primary underline-offset-2 hover:underline"
                          aria-label={t('log.row.aria', { kode: row.kode_transaksi })}
                        >
                          {row.kode_transaksi}
                        </Link>
                      </td>
                      <td className="typescale-body-medium px-4">{row.client_nama}</td>
                      <td className="typescale-body-medium px-4">{row.nik}</td>
                      <td className="typescale-body-medium px-4">
                        <StatusLabel tone={accessTone(row.status_akses)}>
                          {t(`enum.status_akses.${row.status_akses}` as MessageId)}
                        </StatusLabel>
                      </td>
                      <td className="px-4">
                        <Button
                          variant="text"
                          onPress={() => setPayloadFor(row)}
                          aria-label={t('nik.page.payload', { kode: row.kode_transaksi })}
                        >
                          {t('nik.page.payload')}
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </ScrollArea>
          </Card>
        )
      ) : null}

      <ReasonDialog
        isOpen={reasonOpen}
        title={t('nik.reason.title')}
        body={t('nik.reason.body')}
        confirmLabel={t('common.action.search')}
        onConfirm={search}
        onCancel={() => setReasonOpen(false)}
      />

      <ReasonDialog
        isOpen={exportOpen}
        title={t('nik.reason.title')}
        body={t('nik.reason.body')}
        // Export reuses the search reason as a starting point; it is still editable per export.
        initialReason={reason}
        confirmLabel={t('nik.page.export')}
        isSubmitting={exporting}
        onConfirm={(reasonDraft) => void runExport(reasonDraft)}
        onCancel={() => setExportOpen(false)}
      />

      {/* The payload's own reason starts from the search reason and stays editable (section 18.3). */}
      <PayloadDialog
        isOpen={payloadFor !== null}
        kodeTransaksi={payloadFor?.kode_transaksi ?? ''}
        initialReason={reason}
        client={client}
        onClose={() => setPayloadFor(null)}
      />
    </section>
  );
}
