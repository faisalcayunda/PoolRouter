import { useMemo, useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-aria-components';
import { today, type DateValue } from '@internationalized/date';
import {
  api,
  type ApiClient,
  type LogBaris,
  type LogCariRequest,
  type LogPage,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Chip } from '../../components/m3/Chip';
import { DateRangePicker } from '../../components/m3/DatePicker';
import { IconButton } from '../../components/m3/IconButton';
import { Pagination } from '../../components/m3/Pagination';
import { Select, SelectItem } from '../../components/m3/Select';
import { Skeleton } from '../../components/m3/Skeleton';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { TextField } from '../../components/m3/TextField';
import { Icon, icons } from '../../icons';
import { useFormatter } from '../../i18n/useFormatter';
import { enumLabel } from '../../i18n/enums';
import { EmptyState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { MaskedNik } from '../common/MaskedNik';
import { ScrollArea } from '../common/ScrollArea';
import { useRole } from '../common/RoleContext';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { accessTone } from './accessTone';
import { PayloadDialog } from './PayloadDialog';

/** Satu-satunya zona waktu layar ini (WIB); rentang tanggal dikirim sebagai hari penuh WIB. */
const ZONE = 'Asia/Jakarta';

/** Rentang dua tanggal; bentuknya sama dengan `RangeValue` milik React Aria. */
interface DayRange {
  start: DateValue;
  end: DateValue;
}

/** Default kunjungan pertama: hari ini (WIB), seperti rentang di frame Penpot. */
function todayRange(): DayRange {
  const day = today(ZONE);
  return { start: day, end: day };
}

/** `queue_id` di API berupa UUID; isian lain dicari sebagai kode transaksi. */
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function buildRequest(term: string, clientId: string | null, range: DayRange): LogCariRequest {
  const body: LogCariRequest = {
    dari: `${range.start.toString()}T00:00:00+07:00`,
    sampai: `${range.end.toString()}T23:59:59+07:00`,
  };
  const q = term.trim();
  if (q) {
    if (UUID.test(q)) body.queue_id = q;
    else body.kode_transaksi = q;
  }
  if (clientId) body.client_id = clientId;
  return body;
}

/** Kode panjang tampil ringkas seperti di frame ("018f…c4e1"); kode lengkap ada di tautan Detail. */
function shortCode(code: string): string {
  return code.length > 12 ? `${code.slice(0, 4)}…${code.slice(-4)}` : code;
}

/** A client offered by the filter. */
interface ClientOption {
  id: string;
  nama: string;
}

/** Baris per halaman; frame Penpot tidak punya pilihan ukuran halaman ("1–25 dari ..."). */
const PAGE_SIZE = 25;

/** Kartu ringkasan di frame; API belum punya agregatnya, jadi nilainya placeholder. */
const KPI_KEYS = ['today', 'success', 'failed', 'latency'] as const;

export interface LogPageScreenProps {
  client?: ApiClient;
  /** `auditor` unlocks the payload column (section 18.3); every other role never sees it. */
  canOpenPayload?: boolean;
  onOpenPayload?: (row: LogBaris) => void;
  /** Read once from a link (`?kode=`) so a code can be deep-linked without a NIK in the URL. */
  initialCode?: string;
}

/**
 * S11 — Log transaksi (operator, auditor), disusun mengikuti frame Penpot "F0 / S11-log-transaksi".
 * Filters go in the POST body so they never reach the access log (section 18.3); the table shows
 * the NIK masked by the server and the operator never gets a payload affordance.
 *
 * Responsive rules (section 4): below 600 px every row becomes a card, in the medium class
 * (600–839 px) the low-priority column (Client) is hidden so the masked NIK and the time still
 * fit, and above 840 px the full table shows. The markup stays a semantic table at every width.
 */
export function LogPageScreen({
  client = api,
  canOpenPayload,
  onOpenPayload,
  initialCode = '',
}: LogPageScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const format = useFormatter();
  const role = useRole();
  // Section 18.3: only an auditor gets the payload column. An explicit prop still wins so the
  // preview harness and tests can pin the state before the session hook lands.
  const payloadAllowed = canOpenPayload ?? role === 'auditor';
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.log'));
  const snackbar = useSnackbar();

  const [term, setTerm] = useState(initialCode);
  const [clientId, setClientId] = useState<string | null>(null);
  const [range, setRange] = useState<DayRange>(todayRange);
  const [payloadFor, setPayloadFor] = useState<LogBaris | null>(null);
  const [page, setPage] = useState(1);

  /**
   * The submitted filters. Client and range apply at once (a select or a calendar pick is a
   * deliberate choice); the search text applies on Enter so typing never fires a request per
   * keystroke. A deep link (`/log/<kode>`) seeds the code as a filter too — filling the field
   * without filtering would show every client's rows for the day and look like the link did nothing.
   */
  const [applied, setApplied] = useState<LogCariRequest>(() =>
    buildRequest(initialCode, null, todayRange()),
  );
  /** Bumped whenever the operator asks for the same filters again, so Enter re-reads the window. */
  const [run, setRun] = useState(0);
  const filters = useQuery<LogPage>(
    (signal) => client.cariLog(applied, { signal }),
    `${JSON.stringify(applied)}#${run}`,
  );

  const rows = useMemo(() => filters.data?.baris ?? [], [filters.data]);

  /**
   * Clients seen so far, for the select and its chip. Filtering to one client shrinks the result to
   * that client, so the list keeps what earlier results showed instead of collapsing to one option.
   */
  const [known, setKnown] = useState<ClientOption[]>([]);
  const missing = rows.filter((r) => !known.some((c) => c.id === r.client_id));
  if (missing.length > 0) {
    const next = [...known];
    for (const row of missing) {
      if (!next.some((c) => c.id === row.client_id))
        next.push({ id: row.client_id, nama: row.client_nama });
    }
    setKnown(next);
  }
  const selectedClient = known.find((c) => c.id === clientId) ?? null;

  const [rangeError, setRangeError] = useState<string | null>(null);

  function submit(event: FormEvent) {
    event.preventDefault();
    setPage(1);
    setApplied(buildRequest(term, clientId, range));
    setRun((n) => n + 1);
  }

  function changeClient(next: string | null) {
    setClientId(next);
    setPage(1);
    setApplied(buildRequest(term, next, range));
  }

  function changeRange(next: DayRange | null) {
    if (!next) return;
    // A picked end before the start never reaches the request; the field says why.
    if (next.end.compare(next.start) < 0) {
      setRangeError(t('common.date.order_error'));
      return;
    }
    setRangeError(null);
    setRange(next);
    setPage(1);
    setApplied(buildRequest(term, clientId, next));
  }

  function clearFilters() {
    setTerm('');
    setClientId(null);
    setRangeError(null);
    setPage(1);
    setApplied(buildRequest('', null, range));
    setRun((n) => n + 1);
  }

  const pageStart = (page - 1) * PAGE_SIZE;
  const visible = rows.slice(pageStart, pageStart + PAGE_SIZE);
  const pageCount = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
  const rangeLabel =
    rows.length === 0
      ? t('common.pagination.page_range', { dari: 0, sampai: 0, total: 0 })
      : t('common.pagination.page_range', {
          dari: pageStart + 1,
          sampai: Math.min(pageStart + PAGE_SIZE, rows.length),
          total: rows.length,
        });

  const hasFilters = term !== '' || clientId !== null;
  const thClass = 'typescale-label-small border-b border-outline-variant px-5 py-3 font-medium';
  const tdClass = 'typescale-body-medium px-5 py-3 align-middle';

  return (
    <section className="flex w-full flex-col gap-6">
      {/* Judul tanpa bingkai fokus: frame Penpot tidak menggambarnya. Fokus programatik tetap
          dipindah ke sini saat layar dibuka supaya pembaca layar mulai dari judul. */}
      <div className="flex max-w-[620px] flex-col gap-2">
        <h1
          ref={titleRef}
          tabIndex={-1}
          className="typescale-headline-small text-on-surface outline-none"
        >
          {t('log.page.title')}
        </h1>
        <p className="typescale-body-medium text-on-surface-variant">{t('log.page.subtitle')}</p>
      </div>

      {/* Kartu ringkasan: API belum punya agregat harian, jadi nilainya placeholder "Menunggu API". */}
      <ul className="grid grid-cols-2 gap-4 expanded:grid-cols-4">
        {KPI_KEYS.map((key) => (
          <li key={key} className="flex">
            <Card variant="outlined" className="flex w-full flex-col gap-2 rounded-large! p-5!">
              <p className="typescale-title-small text-on-surface-variant">{t(`log.kpi.${key}`)}</p>
              <p className="typescale-headline-small text-on-surface" aria-hidden="true">
                —
              </p>
              <p className="typescale-body-medium text-on-surface-variant">
                {t('log.kpi.pending')}
              </p>
            </Card>
          </li>
        ))}
      </ul>

      <Card variant="outlined" className="flex flex-col rounded-large! p-0!">
        <form
          className="flex flex-col gap-3 p-5 expanded:flex-row expanded:items-start"
          onSubmit={submit}
        >
          {/* Frame Penpot tidak punya tombol "Cari": Enter di kolom cari mengirim form. Tombol
              tersembunyi ini menjadikan pengiriman dengan Enter pasti, apa pun isi form-nya. */}
          <button type="submit" hidden />
          <TextField
            label={t('common.action.search')}
            placeholder={t('log.filter.search_placeholder')}
            value={term}
            onChange={setTerm}
            className="expanded:w-80"
          />
          <Select
            label={t('log.filter.client')}
            selectedKey={clientId ?? 'semua'}
            onSelectionChange={(key) => {
              if (key === null) return;
              changeClient(String(key) === 'semua' ? null : String(key));
            }}
            className="expanded:w-60 [&>button]:w-full"
          >
            {/* A select whose empty state has no option of its own shows the framework's English
                "Select an item"; the catalog names the "all" option. */}
            <SelectItem id="semua">{t('log.filter.client_all')}</SelectItem>
            {known.map((c) => (
              <SelectItem key={c.id} id={c.id}>
                {c.nama}
              </SelectItem>
            ))}
          </Select>
          <div className="flex flex-col gap-1 expanded:w-[280px]">
            <DateRangePicker
              label={t('log.filter.range')}
              calendarLabel={t('log.filter.range_picker')}
              separator="–"
              value={range}
              onChange={changeRange}
              isInvalid={rangeError !== null}
            />
            {rangeError ? (
              <p role="alert" className="typescale-body-small text-error">
                {rangeError}
              </p>
            ) : null}
          </div>
          <Button variant="outlined" onPress={clearFilters} className="expanded:mt-2 self-start">
            {t('common.action.clear_filters')}
          </Button>
        </form>

        {selectedClient ? (
          <div className="flex flex-wrap items-center gap-2 px-5 pb-4">
            <Chip
              variant="input"
              onRemove={() => changeClient(null)}
              removeLabel={t('common.action.clear_filters')}
            >
              {t('log.chip.client', { nama: selectedClient.nama })}
            </Chip>
          </div>
        ) : null}

        <div className="border-t border-surface-container-highest">
          {filters.status === 'loading' ? (
            <div className="flex flex-col gap-2 p-5">
              <p className="sr-only">{t('common.state.loading')}</p>
              {[0, 1, 2, 3, 4].map((i) => (
                <Skeleton key={i} className="h-10 w-full rounded-extra-small" />
              ))}
            </div>
          ) : null}

          {filters.status === 'error' ? (
            <div className="p-5">
              <QueryError error={filters.error} onRetry={filters.reload} />
            </div>
          ) : null}

          {filters.status === 'success' && rows.length === 0 ? (
            <>
              <EmptyState title={t('common.empty.logs.title')} body={t('common.empty.logs.body')} />
              {hasFilters ? (
                <div className="px-5 pb-5">
                  <Button variant="text" onPress={clearFilters}>
                    {t('common.action.clear_filters')}
                  </Button>
                </div>
              ) : null}
            </>
          ) : null}

          {rows.length > 0 ? (
            <>
              <ScrollArea label={t('log.page.title')}>
                <div className="table-card">
                  <table className="w-full border-collapse text-left">
                    <caption className="sr-only">{t('log.page.title')}</caption>
                    <thead>
                      <tr className="bg-surface text-on-surface-variant">
                        <th scope="col" className={thClass}>
                          {t('log.col.time')}
                        </th>
                        <th scope="col" className={thClass}>
                          {t('log.col.code')}
                        </th>
                        <th scope="col" className={`hide-md ${thClass}`}>
                          {t('log.col.client')}
                        </th>
                        <th scope="col" className={thClass}>
                          {t('log.col.nik')}
                        </th>
                        <th scope="col" className={thClass}>
                          {t('log.col.access')}
                        </th>
                        <th scope="col" className={thClass}>
                          <span className="sr-only">{t('log.action.detail')}</span>
                        </th>
                        {payloadAllowed ? (
                          <th scope="col" className={thClass}>
                            {t('log.payload.open')}
                          </th>
                        ) : null}
                      </tr>
                    </thead>
                    <tbody>
                      {visible.map((row) => {
                        const payloadRowId = `payload-${row.kode_transaksi}`;
                        return (
                          <tr
                            key={row.kode_transaksi}
                            className="h-14 border-b border-surface-container text-on-surface"
                          >
                            <td
                              data-label={t('log.col.time')}
                              className={`${tdClass} text-on-surface-variant`}
                            >
                              {format.tanggal(row.requested_at)}, {format.jam(row.requested_at)}
                            </td>
                            <td data-label={t('log.col.code')} className={tdClass}>
                              <span title={row.kode_transaksi}>
                                {shortCode(row.kode_transaksi)}
                              </span>
                            </td>
                            <td data-label={t('log.col.client')} className={`hide-md ${tdClass}`}>
                              {row.client_nama}
                            </td>
                            <td data-label={t('log.col.nik')} className={tdClass}>
                              <MaskedNik value={row.nik} />
                            </td>
                            <td data-label={t('log.col.access')} className={tdClass}>
                              <StatusLabel tone={accessTone(row.status_akses)}>
                                {enumLabel(i18n, 'status_akses', row.status_akses)}
                              </StatusLabel>
                            </td>
                            {/* Tombol teks "Detail" di frame; namanya memuat kode lengkap. */}
                            <td className={`${tdClass} expanded:text-right`}>
                              <Link
                                href={`/log/${encodeURIComponent(row.kode_transaksi)}`}
                                aria-label={t('log.row.aria', { kode: row.kode_transaksi })}
                                className="m3-state m3-focus typescale-label-large inline-flex h-10 items-center justify-center rounded-full px-3 text-primary"
                              >
                                {t('log.action.detail')}
                              </Link>
                            </td>
                            {/* The action cell carries no data-label: its single button names itself. */}
                            {payloadAllowed ? (
                              <td className={tdClass}>
                                <IconButton
                                  aria-label={`${t('log.payload.open')} — ${row.kode_transaksi}`}
                                  aria-describedby={payloadRowId}
                                  onPress={() =>
                                    onOpenPayload ? onOpenPayload(row) : setPayloadFor(row)
                                  }
                                >
                                  <Icon icon={icons.visibility} />
                                </IconButton>
                                <span id={payloadRowId} className="sr-only">
                                  {row.kode_transaksi}
                                </span>
                              </td>
                            ) : null}
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              </ScrollArea>

              <Pagination
                variant="text"
                className="border-t border-surface-container-highest p-5"
                page={page}
                pageCount={pageCount}
                onPageChange={setPage}
                prevLabel={t('log.page.prev')}
                nextLabel={t('log.page.next')}
                label={t('common.pagination.page_number', { ke: page, total: pageCount })}
                rangeText={rangeLabel}
              />
            </>
          ) : null}
        </div>
      </Card>

      {/* Section 7: an operator is told where a full-NIK search lives instead of hunting for it. */}
      {payloadAllowed ? null : (
        <p className="typescale-body-medium text-on-surface-variant">{t('log.page.nik_hint')}</p>
      )}

      {/* Section 18.3: the opening is confirmed in a snackbar and recorded in the access audit. */}
      <PayloadDialog
        isOpen={payloadFor !== null}
        kodeTransaksi={payloadFor?.kode_transaksi ?? ''}
        // Section 18.3: every opening needs its own reason — nothing carries over from S11.
        initialReason=""
        client={client}
        onOpened={() => snackbar.notify(t('log.payload.snackbar.opened'))}
        onClose={() => setPayloadFor(null)}
      />
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
