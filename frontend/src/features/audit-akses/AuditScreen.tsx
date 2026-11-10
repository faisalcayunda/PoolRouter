import { useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type AuditAksesPage, type AuditCariRequest } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Pagination } from '../../components/m3/Pagination';
import { Select, SelectItem } from '../../components/m3/Select';
import { Skeleton } from '../../components/m3/Skeleton';
import { StatusLabel, type StatusLabelTone } from '../../components/m3/StatusLabel';
import { TextField } from '../../components/m3/TextField';
import idMessages from '../../i18n/messages/id.json';
import { useFormatter } from '../../i18n/useFormatter';
import { enumLabel } from '../../i18n/enums';
import {
  DateRangeField,
  formatTypedDate,
  parseTypedDate,
  type DateRange,
  type DateRangeFieldErrors,
} from '../common/DateRangeField';
import { PageHeader, EmptyState, QueryError } from '../common/page';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { useQuery } from '../common/useQuery';
import { ScrollArea } from '../common/ScrollArea';

function lastDay(): DateRange {
  const now = new Date();
  const start = new Date(now.getTime() - 24 * 3600 * 1000);
  const iso = (d: Date) => {
    const wib = new Date(d.getTime() + 7 * 3600 * 1000);
    return { date: wib.toISOString().slice(0, 10), time: wib.toISOString().slice(11, 16) };
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

/** Actions the catalog can name; the filter offers exactly these (`enum.aksi_audit`). */
const AKSI_VALUES = Object.keys(idMessages.enum.aksi_audit);

/** Cell classes shared with the other v2 tables (S08 client list). */
const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';
const PLACEHOLDER = '—';

/** Chip tone per action (frame S13: Cari info, Ditolak error, Ekspor warning). */
const AKSI_TONE: Record<string, StatusLabelTone> = {
  cari_nik: 'info',
  ditolak: 'error',
  ekspor: 'warning',
};
/** `StatusLabel` has no warning-container tone (its `warning` reuses error-container), so Ekspor overrides it. */
const WARNING_CHIP = 'bg-warning-container! text-on-warning-container!';

export interface AuditScreenProps {
  client?: ApiClient;
}

/**
 * S13 — Audit akses. Read-only: no edit or delete action exists (section 7).
 *
 * Columns follow the Penpot frame: time, user, role, action chip, filter and record count. The
 * admin contract of `searchAuditAkses` only returns time, user id, action, result and reason, so role,
 * filter and record count render as "—" until the API carries them, and the reason is not shown.
 */
export function AuditScreen({ client = api }: AuditScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const format = useFormatter();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.audit'));

  const initial = useMemo(() => lastDay(), []);
  const [range, setRange] = useState<DateRange>(initial);
  const [typed, setTyped] = useState({
    start: formatTypedDate(initial.startDate),
    end: formatTypedDate(initial.endDate),
  });
  const [errors, setErrors] = useState<DateRangeFieldErrors>({});
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [query, setQuery] = useState<AuditCariRequest>({ dari: initial.from, sampai: initial.to });

  const [aksi, setAksi] = useState('semua');
  const [pengguna, setPengguna] = useState('');
  // One entry per page visited: `[null]` is page 1, later entries are the `kursor` of the previous
  // response. Applying a new filter or range starts the trail over.
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const page = cursors.length;
  const cursor = cursors[cursors.length - 1] ?? null;
  const request: AuditCariRequest = cursor ? { ...query, kursor: cursor } : query;

  const audit = useQuery<AuditAksesPage>(
    (signal) => client.cariAuditAkses(request, { signal }),
    JSON.stringify(request),
  );

  const rows = audit.data?.baris ?? [];

  function submit(event: React.FormEvent) {
    event.preventDefault();
    const next: DateRangeFieldErrors = {};
    const startIso = parseTypedDate(typed.start);
    const endIso = parseTypedDate(typed.end);
    if (!startIso) next.startDate = t('common.date.format_error');
    if (!endIso) next.endDate = t('common.date.format_error');
    setErrors(next);
    if (!startIso || !endIso) {
      setRangeError(null);
      return;
    }
    // The range itself is the query: the date fields are the source of truth on submit.
    const from = range.startTime ? `${startIso}T${range.startTime}:00+07:00` : '';
    const to = range.endTime ? `${endIso}T${range.endTime}:59+07:00` : '';
    if (!from || !to || from > to) {
      setRangeError(t('common.date.order_error'));
      return;
    }
    setRangeError(null);
    const userId = pengguna.trim();
    setQuery({
      dari: from,
      sampai: to,
      ...(aksi === 'semua' ? {} : { aksi }),
      ...(userId ? { pengguna_id: userId } : {}),
    });
    setCursors([null]);
  }

  return (
    <section className="flex w-full flex-col gap-6">
      <PageHeader
        title={t('audit.page.title')}
        subtitle={t('audit.page.subtitle')}
        titleRef={titleRef}
      />

      <form className="flex flex-col gap-4" onSubmit={submit}>
        <div className="grid grid-cols-1 gap-4 expanded:grid-cols-2">
          <TextField
            label={t('audit.filter.user')}
            value={pengguna}
            autoComplete="off"
            onChange={setPengguna}
          />
          <Select
            label={t('audit.filter.action')}
            selectedKey={aksi}
            onSelectionChange={(key) => {
              if (key !== null) setAksi(String(key));
            }}
          >
            <SelectItem id="semua">{t('common.action.all')}</SelectItem>
            {AKSI_VALUES.map((value) => (
              <SelectItem key={value} id={value}>
                {enumLabel(i18n, 'aksi_audit', value)}
              </SelectItem>
            ))}
          </Select>
          <div className="expanded:col-span-2">
            <DateRangeField
              legend={t('audit.filter.range')}
              value={range}
              typed={typed}
              onTypedChange={setTyped}
              onChange={(next) => {
                if (next) setRange(next);
                setRangeError(null);
              }}
              errors={errors}
              errorMessage={rangeError}
            />
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button type="submit">{t('common.action.search')}</Button>
        </div>
      </form>

      {audit.status === 'loading' ? (
        <Card variant="outlined" className="flex flex-col gap-2">
          <p className="sr-only">{t('common.state.loading')}</p>
          {[0, 1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-10 w-full rounded-extra-small" />
          ))}
        </Card>
      ) : null}

      {audit.status === 'error' ? <QueryError error={audit.error} onRetry={audit.reload} /> : null}

      {audit.status === 'success' && rows.length === 0 ? (
        <EmptyState title={t('audit.empty.title')} body={t('audit.empty.body')} />
      ) : null}

      {rows.length > 0 ? (
        <Card variant="outlined" className="p-0">
          <ScrollArea label={t('audit.page.title')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('audit.page.title')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('audit.col.time')}
                    </th>
                    <th scope="col" className={`hide-md ${TH}`}>
                      {t('audit.col.user')}
                    </th>
                    <th scope="col" className={`hide-md ${TH}`}>
                      {t('audit.col.role')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('audit.col.action')}
                    </th>
                    <th scope="col" className={`hide-md ${TH}`}>
                      {t('audit.col.filter')}
                    </th>
                    <th scope="col" className={`${TH} text-right`}>
                      {t('audit.col.count')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr
                      key={row.audit_id}
                      className="border-b border-outline-variant text-on-surface"
                    >
                      <td data-label={t('audit.col.time')} className={TD}>
                        {format.tanggalWaktu(row.dilakukan_pada)}
                      </td>
                      <td data-label={t('audit.col.user')} className={`hide-md ${TD}`}>
                        {/* Section 13.2: the raw id is an identifier, labelled as one — never a name. */}
                        <span className="break-all font-code text-on-surface-variant">
                          {row.pengguna_id}
                        </span>
                      </td>
                      {/* Gap: the audit contract carries no role, filter or record count yet; the frame's columns stay as placeholders. */}
                      <td data-label={t('audit.col.role')} className={`hide-md ${TD}`}>
                        {PLACEHOLDER}
                      </td>
                      <td data-label={t('audit.col.action')} className={TD}>
                        <div className="flex flex-wrap gap-1">
                          {/* `enumLabel` keeps an action outside the catalog from rendering as a raw key. */}
                          <StatusLabel
                            tone={AKSI_TONE[row.aksi] ?? 'neutral'}
                            className={row.aksi === 'ekspor' ? WARNING_CHIP : undefined}
                          >
                            {enumLabel(i18n, 'aksi_audit', row.aksi)}
                          </StatusLabel>
                          {/* A refused attempt under another action still reads as refused. */}
                          {row.hasil === 'ditolak' && row.aksi !== 'ditolak' ? (
                            <StatusLabel tone="error">
                              {enumLabel(i18n, 'hasil', row.hasil)}
                            </StatusLabel>
                          ) : null}
                        </div>
                      </td>
                      <td data-label={t('audit.col.filter')} className={`hide-md ${TD}`}>
                        {PLACEHOLDER}
                      </td>
                      <td data-label={t('audit.col.count')} className={`${TD} expanded:text-right`}>
                        {PLACEHOLDER}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        </Card>
      ) : null}

      {audit.status === 'success' && (page > 1 || audit.data.kursor) ? (
        <div className="flex justify-end">
          <Pagination
            page={page}
            pageCount={audit.data.kursor ? page + 1 : page}
            onPageChange={(next) => {
              if (next < page) setCursors((trail) => trail.slice(0, -1));
              else if (audit.data.kursor) setCursors((trail) => [...trail, audit.data.kursor!]);
            }}
            prevLabel={t('common.pagination.prev_page')}
            nextLabel={t('common.pagination.next_page')}
            label={t('audit.page.title')}
            rangeText={t('common.pagination.page_current', { ke: page })}
          />
        </div>
      ) : null}

      <p className="typescale-body-small text-on-surface-variant">{t('audit.filter.note')}</p>
    </section>
  );
}
