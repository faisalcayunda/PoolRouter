import { useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-aria-components';
import {
  api,
  type ApiClient,
  type LogBaris,
  type LogCariRequest,
  type LogPage,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Card } from '../../components/m3/Card';
import { Chip } from '../../components/m3/Chip';
import { IconButton } from '../../components/m3/IconButton';
import { Pagination } from '../../components/m3/Pagination';
import { Select, SelectItem } from '../../components/m3/Select';
import { Skeleton } from '../../components/m3/Skeleton';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { TextField } from '../../components/m3/TextField';
import { Icon, icons } from '../../icons';
import { useFormatter } from '../../i18n/useFormatter';
import { enumLabel } from '../../i18n/enums';
import {
  DateRangeField,
  formatTypedDate,
  validateTypedRange,
  type DateRange,
  type DateRangeFieldErrors,
} from '../common/DateRangeField';
import { PageHeader, EmptyState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { ScrollArea } from '../common/ScrollArea';
import { useRole } from '../common/RoleContext';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { accessTone } from './accessTone';
import { PayloadDialog } from './PayloadDialog';

/** Default window a first visit shows: the last 24 hours, so the table is never silently empty. */
function lastDay(): DateRange {
  const now = new Date();
  const start = new Date(now.getTime() - 24 * 3600 * 1000);
  const iso = (d: Date) => {
    const wib = new Date(d.getTime() + 7 * 3600 * 1000);
    return {
      date: wib.toISOString().slice(0, 10),
      time: wib.toISOString().slice(11, 16),
    };
  };
  const s = iso(start);
  const e = iso(now);
  return {
    from: `${s.date}T${s.time}:00+07:00`,
    to: `${e.date}T${e.time}:59+07:00`,
    startDate: s.date,
    startTime: s.time,
    endDate: e.date,
    endTime: e.time,
  };
}

/** A client offered by the filter; the list comes from the rows already loaded. */
interface ClientOption {
  id: string;
  nama: string;
}

/** How many rows one page shows (section 13.2: "Baris per halaman" select). */
const PAGE_SIZES = [10, 25, 50, 100];

export interface LogPageScreenProps {
  client?: ApiClient;
  /** `auditor` unlocks the payload column (section 18.3); every other role never sees it. */
  canOpenPayload?: boolean;
  onOpenPayload?: (row: LogBaris) => void;
  /** Read once from a link (`?kode=`) so a code can be deep-linked without a NIK in the URL. */
  initialCode?: string;
}

/**
 * S11 — Log transaksi (operator, auditor). Filters go in the POST body so they never reach the
 * access log (section 18.3); the table shows the NIK masked by the server and the operator never
 * gets a payload affordance.
 *
 * Responsive rules (section 4): below 600 px every row becomes a card, in the medium class
 * (600–839 px) the low-priority columns (queue_id, Client) are hidden so the masked NIK and the
 * time still fit, and above 840 px the full table shows. The markup stays a semantic table at
 * every width — `display: block` on the table only changes how it wraps, not what assistive tech
 * sees.
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

  const [code, setCode] = useState(initialCode);
  const [queueId, setQueueId] = useState('');
  const [clientId, setClientId] = useState<string | null>(null);
  const [range, setRange] = useState<DateRange>(lastDay);
  const [typed, setTyped] = useState(() => {
    const r = lastDay();
    return { start: formatTypedDate(r.startDate), end: formatTypedDate(r.endDate) };
  });
  const [codeError, setCodeError] = useState<string | null>(null);
  const [rangeErrors, setRangeErrors] = useState<DateRangeFieldErrors>({});
  const [rangeMessage, setRangeMessage] = useState<string | null>(null);
  const [payloadFor, setPayloadFor] = useState<LogBaris | null>(null);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(25);

  /**
   * The submitted filters. Only the range changes it on its own: a new window should show results
   * immediately, while typing in a text field must not fire a request per keystroke. A deep link
   * (`/log/<kode>`) seeds the code as a filter too — filling the field without filtering would show
   * every client's rows for the window and look like the link did nothing.
   */
  const [applied, setApplied] = useState<LogCariRequest>(() => {
    const initial = lastDay();
    const seeded: LogCariRequest = { dari: initial.from, sampai: initial.to };
    const linked = initialCode.trim();
    if (linked) seeded.kode_transaksi = linked;
    return seeded;
  });
  /**
   * Bumped whenever the operator asks for the same filters again. The key carries it, so "Cari" on
   * an unchanged form re-reads the window instead of silently keeping the previous snapshot.
   */
  const [run, setRun] = useState(0);
  const filters = useQuery<LogPage>(
    (signal) => client.cariLog(applied, { signal }),
    `${JSON.stringify(applied)}#${run}`,
  );
  const appliedRange = `${applied.dari}|${applied.sampai}`;

  // Track the range the query last ran with, and re-run once when the pickers move.
  const [rangeInUse, setRangeInUse] = useState(appliedRange);
  if (`${range.from}|${range.to}` !== rangeInUse) {
    const nextFrom = range.from;
    const nextTo = range.to;
    setRangeInUse(`${nextFrom}|${nextTo}`);
    if (nextFrom && nextTo)
      setApplied((current) => ({ ...current, dari: nextFrom, sampai: nextTo }));
  }

  const rows = useMemo(() => filters.data?.baris ?? [], [filters.data]);

  /** Clients seen in this result, for the client filter and its chip (section 7). */
  const clients = useMemo<ClientOption[]>(() => {
    const seen = new Map<string, string>();
    for (const row of rows) seen.set(row.client_id, row.client_nama);
    return [...seen].map(([id, nama]) => ({ id, nama }));
  }, [rows]);

  const selectedClient = clients.find((c) => c.id === clientId) ?? null;

  function submit(event: React.FormEvent) {
    event.preventDefault();
    // Section 15.1: both ends are validated before anything is sent, and the messages are per
    // field so the reader can see which date is wrong. Without this an inverted or half-typed
    // range was silently ignored and the previous window's rows stayed on screen.
    const checked = validateTypedRange(range, typed, {
      format: t('common.date.format_error'),
      order: t('common.date.order_error'),
    });
    setRangeErrors(checked.errors);
    setRangeMessage(checked.errorMessage);

    // The code field is short; a code with spaces is almost certainly a paste slip.
    if (code.trim() !== '' && /\s/.test(code.trim())) {
      setCodeError(t('log.filter.code_help'));
      return;
    }
    setCodeError(null);
    if (!checked.ok) return;
    setPage(1);
    const next: LogCariRequest = { dari: range.from, sampai: range.to };
    if (code.trim()) next.kode_transaksi = code.trim();
    if (queueId.trim()) next.queue_id = queueId.trim();
    if (clientId) next.client_id = clientId;
    setApplied(next);
    setRun((n) => n + 1);
  }

  function clearFilters() {
    const base: LogCariRequest = { dari: range.from, sampai: range.to };
    setCode('');
    setQueueId('');
    setClientId(null);
    setCodeError(null);
    setRangeErrors({});
    setRangeMessage(null);
    setPage(1);
    setApplied(base);
    setRun((n) => n + 1);
  }

  const pageStart = (page - 1) * pageSize;
  const visible = rows.slice(pageStart, pageStart + pageSize);
  const pageCount = Math.max(1, Math.ceil(rows.length / pageSize));
  const rangeLabel =
    rows.length === 0
      ? t('common.pagination.page_range', { dari: 0, sampai: 0, total: 0 })
      : t('common.pagination.page_range', {
          dari: pageStart + 1,
          sampai: Math.min(pageStart + pageSize, rows.length),
          total: rows.length,
        });

  const hasFilters = code !== '' || queueId !== '' || clientId !== null;

  return (
    <section className="flex w-full flex-col gap-6">
      <PageHeader title={t('log.page.title')} subtitle={t('log.page.subtitle')} titleRef={titleRef}>
        {/* Section 7: an operator is told where a full-NIK search lives instead of hunting for it. */}
        {payloadAllowed ? null : (
          <InlineAlert tone="info" live="polite">
            {t('log.page.nik_hint')}
          </InlineAlert>
        )}
      </PageHeader>

      {/* aria-label and no `form` role kept out of the landmark list: a named form would add a
          landmark per screen (non-blocking note in the SHAA-203 design review). */}
      <form className="flex flex-col gap-4" onSubmit={submit}>
        <div className="flex flex-col gap-4 md:flex-row">
          <TextField
            label={t('log.filter.code')}
            description={t('log.filter.code_help')}
            value={code}
            onChange={setCode}
            isInvalid={codeError !== null}
            errorMessage={codeError ?? undefined}
            className="md:flex-1"
          />
          <TextField
            label={t('log.filter.queue')}
            description={t('log.filter.queue_help')}
            value={queueId}
            onChange={setQueueId}
            className="md:flex-1"
          />
          <Select
            label={t('log.filter.client')}
            selectedKey={clientId ?? 'semua'}
            onSelectionChange={(key) => {
              if (key === null) return;
              setClientId(String(key) === 'semua' ? null : String(key));
              setPage(1);
            }}
            className="md:w-64"
          >
            {/* A select whose empty state has no option of its own shows the framework's English
                "Select an item"; the catalog already names "all" (common.action.all). */}
            <SelectItem id="semua">{t('common.action.all')}</SelectItem>
            {clients.map((c) => (
              <SelectItem key={c.id} id={c.id}>
                {c.nama}
              </SelectItem>
            ))}
          </Select>
        </div>
        <DateRangeField
          legend={t('log.filter.range')}
          value={range}
          typed={typed}
          onTypedChange={setTyped}
          errors={rangeErrors}
          errorMessage={rangeMessage}
          onChange={(next) => {
            if (!next) return;
            setRange(next);
            setRangeErrors({});
            setRangeMessage(null);
          }}
        />
        <div className="flex flex-wrap gap-2">
          <Button type="submit">{t('common.action.search')}</Button>
          <Button variant="text" onPress={clearFilters}>
            {t('common.action.clear_filters')}
          </Button>
        </div>
      </form>

      {selectedClient ? (
        <div className="flex flex-wrap items-center gap-2">
          <Chip
            variant="input"
            onRemove={() => {
              setClientId(null);
              setPage(1);
            }}
            removeLabel={t('common.action.clear_filters')}
          >
            {t('log.chip.client', { nama: selectedClient.nama })}
          </Chip>
        </div>
      ) : null}

      {filters.status === 'loading' ? (
        <Card variant="outlined" className="flex flex-col gap-2">
          <p className="sr-only">{t('common.state.loading')}</p>
          {[0, 1, 2, 3, 4].map((i) => (
            <Skeleton key={i} className="h-10 w-full rounded-extra-small" />
          ))}
        </Card>
      ) : null}

      {filters.status === 'error' ? (
        <QueryError error={filters.error} onRetry={filters.reload} />
      ) : null}

      {filters.status === 'success' && rows.length === 0 ? (
        <>
          <EmptyState title={t('common.empty.logs.title')} body={t('common.empty.logs.body')} />
          {hasFilters ? (
            <div>
              <Button variant="text" onPress={clearFilters}>
                {t('common.action.clear_filters')}
              </Button>
            </div>
          ) : null}
        </>
      ) : null}

      {rows.length > 0 ? (
        <>
          <Card variant="outlined" className="p-0">
            <ScrollArea label={t('log.page.title')}>
              <div className="table-card">
                <table className="w-full border-collapse text-left">
                  <caption className="sr-only">{t('log.page.title')}</caption>
                  <thead>
                    <tr className="typescale-title-small text-on-surface-variant">
                      <th
                        scope="col"
                        className="border-b border-outline-variant px-4 py-3 font-medium"
                      >
                        {t('log.col.time')}
                      </th>
                      <th
                        scope="col"
                        className="border-b border-outline-variant px-4 py-3 font-medium"
                      >
                        {t('log.col.code')}
                      </th>
                      <th
                        scope="col"
                        className="hide-md border-b border-outline-variant px-4 py-3 font-medium"
                      >
                        {t('log.filter.queue')}
                      </th>
                      <th
                        scope="col"
                        className="hide-md border-b border-outline-variant px-4 py-3 font-medium"
                      >
                        {t('log.col.client')}
                      </th>
                      <th
                        scope="col"
                        className="border-b border-outline-variant px-4 py-3 font-medium"
                      >
                        {t('log.col.nik')}
                      </th>
                      <th
                        scope="col"
                        className="border-b border-outline-variant px-4 py-3 font-medium"
                      >
                        {t('log.col.access')}
                      </th>
                      <th
                        scope="col"
                        className="border-b border-outline-variant px-4 py-3 font-medium"
                      >
                        {t('log.col.requested_at')}
                      </th>
                      {payloadAllowed ? (
                        <th
                          scope="col"
                          className="border-b border-outline-variant px-4 py-3 font-medium"
                        >
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
                          className="border-b border-outline-variant text-on-surface"
                        >
                          <td
                            data-label={t('log.col.time')}
                            className="typescale-body-medium px-4 py-3 align-middle"
                          >
                            {format.tanggalWaktu(row.requested_at)}
                          </td>
                          <td
                            data-label={t('log.col.code')}
                            className="typescale-body-medium px-4 py-3 align-middle"
                          >
                            <Link
                              href={`/log/${encodeURIComponent(row.kode_transaksi)}`}
                              className="m3-focus text-primary underline-offset-2 hover:underline"
                            >
                              {row.kode_transaksi}
                            </Link>
                          </td>
                          <td
                            data-label={t('log.filter.queue')}
                            className="hide-md typescale-body-medium px-4 py-3 align-middle"
                          >
                            {row.queue_id}
                          </td>
                          <td
                            data-label={t('log.col.client')}
                            className="hide-md typescale-body-medium px-4 py-3 align-middle"
                          >
                            {row.client_nama}
                          </td>
                          <td
                            data-label={t('log.col.nik')}
                            className="typescale-body-medium px-4 py-3 align-middle"
                          >
                            {row.nik}
                          </td>
                          <td
                            data-label={t('log.col.access')}
                            className="typescale-body-medium px-4 py-3 align-middle"
                          >
                            <StatusLabel tone={accessTone(row.status_akses)}>
                              {enumLabel(i18n, 'status_akses', row.status_akses)}
                            </StatusLabel>
                          </td>
                          <td
                            data-label={t('log.col.requested_at')}
                            className="typescale-body-medium px-4 py-3 align-middle"
                          >
                            {format.tanggalWaktu(row.requested_at)}
                          </td>
                          {/* The action cell carries no data-label: its single button names itself. */}
                          {payloadAllowed ? (
                            <td className="typescale-body-medium px-4 py-3 align-middle">
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
          </Card>

          <div className="flex flex-wrap items-center justify-between gap-4">
            <Select
              label={t('common.pagination.rows_per_page')}
              selectedKey={String(pageSize)}
              onSelectionChange={(key) => {
                if (key === null) return;
                setPageSize(Number(key));
                setPage(1);
              }}
              className="w-56"
            >
              {PAGE_SIZES.map((n) => (
                <SelectItem key={String(n)} id={String(n)}>
                  {String(n)}
                </SelectItem>
              ))}
            </Select>
            <Pagination
              page={page}
              pageCount={pageCount}
              onPageChange={setPage}
              prevLabel={t('common.pagination.prev_page')}
              nextLabel={t('common.pagination.next_page')}
              label={`${t('common.pagination.page_number', { ke: page, total: pageCount })}`}
              rangeText={rangeLabel}
            />
          </div>
        </>
      ) : null}

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
