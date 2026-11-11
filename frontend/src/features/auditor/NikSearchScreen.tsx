import { useEffect, useRef, useState, type ReactNode } from 'react';
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
import { DateRangePicker } from '../../components/m3/DatePicker';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Icon, icons } from '../../icons';
import { Card } from '../../components/m3/Card';
import { Pagination } from '../../components/m3/Pagination';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { TextField } from '../../components/m3/TextField';
import { useFormatter } from '../../i18n/useFormatter';
import { dayRangeBounds, todayRange, type DayRange } from '../common/dayRange';
import { ReasonDialog } from '../common/ReasonDialog';
import { FilterBar } from '../common/layout';
import { PageHeader, EmptyState, LoadingState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { MaskedNik } from '../common/MaskedNik';
import { ScrollArea } from '../common/ScrollArea';
import { accessTone } from '../log/accessTone';
import { PayloadDialog } from '../log/PayloadDialog';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { enumLabel } from '../../i18n/enums';

const NIK_LENGTH = 16;

/** Table cell classes shared with the other v2 tables (S08 client list). */
const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

export interface NikSearchScreenProps {
  client?: ApiClient;
}

/**
 * S12 — Pencarian log per NIK (auditor only). The NIK is mandatory and exactly 16 digits, the
 * reason dialog comes before the request, and every page turn repeats the reason so the audit row
 * matches the page the auditor actually saw (ADR-0003 rev 3 3.6a–c).
 */
export function NikSearchScreen({ client = api }: NikSearchScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const format = useFormatter();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.nik_search'));
  const snackbar = useSnackbar();

  const [nik, setNik] = useState('');
  const [nikTouched, setNikTouched] = useState(false);
  const [range, setRange] = useState<DayRange | null>(todayRange);
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
  /**
   * The submitted request whose success snackbar has already been shown. Tracked by identity, not
   * by key: repeating the same search is a fresh audited action, so it gets its own confirmation.
   */
  const loggedRequest = useRef<unknown>(null);
  /**
   * Bumped on every accepted search. The query key includes it, so submitting the same filters
   * again really does call the API again instead of reusing the previous result: a repeat search is
   * a new audited action, and the snapshot it returns may have moved on.
   */
  const [run, setRun] = useState(0);
  /**
   * Cursor trail, one entry per page visited: `[null]` is page 1, and each later entry is the
   * `kursor` the previous response returned. A page turn only moves along this trail and re-sends the
   * submitted body (same `alasan`), so it never reopens the reason dialog; a fresh search resets it.
   */
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const page = cursors.length;
  const cursor = cursors[cursors.length - 1] ?? null;

  const nikValid = /^\d{16}$/.test(nik.trim());
  const nikError = nikTouched && !nikValid ? t('nik.page.error') : null;

  const query = useQuery<LogPage>(
    (signal) =>
      client.cariLogNik(cursor ? { ...submitted!.body, kursor: cursor } : submitted!.body, {
        signal,
        alasan: submitted!.body.alasan,
      }),
    `${submitted?.key ?? 'idle'}#${run}#${cursors.length}:${cursor ?? ''}`,
    { enabled: submitted !== null },
  );

  function requestReason() {
    setNikTouched(true);
    if (!nikValid) return;
    if (!range) {
      // The note above the form explains that a NIK is required; the range needs its own line.
      setRangeError(t('common.date.only_note'));
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
      ...dayRangeBounds(range!),
      alasan: nextReason,
    };
    setSubmitted({ body, key: JSON.stringify(body) });
    setCursors([null]);
    setRun((n) => n + 1);
  }

  /**
   * Section 7: the search is an audited action, so its success is confirmed in a snackbar — but
   * only once the request that records it came back. Announcing it at submit time claimed a
   * failed search had been written to the audit trail. One confirmation per submitted search.
   */
  useEffect(() => {
    if (query.status !== 'success' || submitted === null) return;
    if (loggedRequest.current === submitted) return;
    loggedRequest.current = submitted;
    snackbar.notify(t('nik.snackbar.logged'));
  }, [query.status, submitted, snackbar, t]);

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
        { nik: nik.trim(), ...dayRangeBounds(range!), alasan },
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

  const searched = query.status === 'success';
  const lastPage = searched && !query.data.kursor;

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={t('nik.page.title')}
        subtitle={t('nik.page.subtitle')}
        titleRef={titleRef}
        actions={
          // Frame S12: "Ekspor hasil" sits top right and only exists once there are results (S12c has none).
          searched ? (
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
          ) : null
        }
      />

      <Card variant="container" className="p-4 large:p-6">
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            requestReason();
          }}
          aria-describedby={nikError ? 'nik-range-note' : undefined}
        >
          <FilterBar className="expanded:items-start">
            <TextField
              className="expanded:w-72"
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
            <div className="flex flex-col gap-1 expanded:w-[280px]">
              <DateRangePicker
                label={t('nik.page.range')}
                calendarLabel={t('log.filter.range_picker')}
                separator="–"
                value={range}
                onChange={(next) => {
                  if (next && next.end.compare(next.start) < 0) {
                    setRangeError(t('common.date.order_error'));
                    return;
                  }
                  setRange(next);
                  setRangeError(null);
                }}
                isInvalid={rangeError !== null}
              />
              {rangeError ? (
                <p role="alert" className="typescale-body-small text-error">
                  {rangeError}
                </p>
              ) : null}
            </div>
            {/* Sejajar dengan kotak isian 40 px: label 20 + jarak 4 di atasnya. */}
            <Button variant="filled" type="submit" className="expanded:mt-6">
              {t('common.action.search')}
            </Button>
          </FilterBar>
          {/* Frame S12c: the rule that a range alone is not enough shows with the NIK error. */}
          {nikError ? (
            <p id="nik-range-note" className="typescale-body-small text-on-surface-variant">
              {t('nik.page.range_only_note')}
            </p>
          ) : null}
        </form>
      </Card>

      {exportNotice === 'too_large' ? (
        <InlineAlert tone="warning" live="polite" heading={t('export.error.title')}>
          {t('export.error.too_large')}
        </InlineAlert>
      ) : null}
      {exportNotice === 'error' ? (
        <InlineAlert tone="error" live="assertive" heading={t('risk.common.action_failed')}>
          {t('error.network')}
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
          <Card variant="container" className="overflow-hidden">
            <ScrollArea label={t('nik.page.title')}>
              <div className="table-card">
                <table className="w-full border-collapse text-left">
                  <caption className="sr-only">{t('nik.page.title')}</caption>
                  <thead>
                    <tr className="typescale-title-small text-on-surface-variant">
                      <th scope="col" className={TH}>
                        {t('log.col.time')}
                      </th>
                      <th scope="col" className={TH}>
                        {t('log.col.nik')}
                      </th>
                      <th scope="col" className={TH}>
                        {t('log.col.access')}
                      </th>
                      <th scope="col" className={`hide-md ${TH}`}>
                        {t('log.col.code')}
                      </th>
                      <th scope="col" className={`hide-md ${TH}`}>
                        {t('log.col.client')}
                      </th>
                      <th scope="col" className={TH}>
                        <span className="sr-only">{t('audit.col.action')}</span>
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((row) => (
                      <tr
                        key={`${row.kode_transaksi}-${row.requested_at}`}
                        className="border-b border-outline-variant text-on-surface"
                      >
                        <td data-label={t('log.col.time')} className={TD}>
                          {format.tanggalWaktu(row.requested_at)}
                        </td>
                        <td data-label={t('log.col.nik')} className={TD}>
                          <MaskedNik value={row.nik} />
                        </td>
                        <td data-label={t('log.col.access')} className={TD}>
                          <StatusLabel tone={accessTone(row.status_akses)}>
                            {enumLabel(i18n, 'status_akses', row.status_akses)}
                          </StatusLabel>
                        </td>
                        <td data-label={t('log.col.code')} className={`hide-md ${TD}`}>
                          <Link
                            href={`/log/${encodeURIComponent(row.kode_transaksi)}`}
                            className="m3-focus rounded-small text-primary-text underline-offset-2 hover:underline"
                            aria-label={t('log.row.aria', { kode: row.kode_transaksi })}
                          >
                            {row.kode_transaksi}
                          </Link>
                        </td>
                        <td data-label={t('log.col.client')} className={`hide-md ${TD}`}>
                          {row.client_nama}
                        </td>
                        {/* Inline action column (as S08/S10): a text button that names its row. */}
                        <td className={TD}>
                          <Button
                            variant="text"
                            aria-label={`${t('log.payload.open')} — ${row.kode_transaksi}`}
                            onPress={() => setPayloadFor(row)}
                          >
                            {t('nik.page.payload')}
                          </Button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </ScrollArea>
          </Card>
        )
      ) : null}

      {searched && rows.length > 0 ? (
        <div className="flex flex-wrap items-center justify-end gap-4">
          {/* Gap: the API is cursor-only (no page-size parameter, no total), so the size shown is the rows on this page. */}
          <p className="typescale-body-small text-on-surface-variant">
            {t('common.pagination.rows_per_page')}: {rows.length}
          </p>
          <Pagination
            page={page}
            pageCount={query.data.kursor ? page + 1 : page}
            onPageChange={(next) => {
              if (next < page) setCursors((trail) => trail.slice(0, -1));
              else if (query.data.kursor) setCursors((trail) => [...trail, query.data.kursor!]);
            }}
            prevLabel={t('common.pagination.prev_page')}
            nextLabel={t('common.pagination.next_page')}
            label={t('nik.page.title')}
            rangeText={
              // The total is only known on the last page of a single-page result.
              lastPage && page === 1
                ? t('common.pagination.page_range', {
                    dari: 1,
                    sampai: rows.length,
                    total: rows.length,
                  })
                : t('common.pagination.page_current', { ke: page })
            }
          />
        </div>
      ) : null}

      {searched ? <InlineAlert tone="info">{t('nik.page.retention')}</InlineAlert> : null}

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
        reasonCopy={{
          body: t('nik.payload.body'),
          fieldLabel: t('nik.reason.field'),
          fieldHint: t('nik.payload.help'),
          confirmLabel: t('nik.payload.open'),
        }}
        onOpened={() => snackbar.notify(t('log.payload.snackbar.opened'))}
        onClose={() => setPayloadFor(null)}
      />
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
