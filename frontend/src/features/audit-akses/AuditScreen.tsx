import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type AuditAksesPage, type AuditCariRequest } from '../../api/client';
import { DateRangePicker } from '../../components/m3/DatePicker';
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
import { dayRangeBounds, todayRange, type DayRange } from '../common/dayRange';
import { FilterBar } from '../common/layout';
import { PageHeader, EmptyState, QueryError } from '../common/page';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { useQuery } from '../common/useQuery';
import { ScrollArea } from '../common/ScrollArea';

/** Actions the catalog can name; the filter offers exactly these (`enum.aksi_audit`). */
const AKSI_VALUES = Object.keys(idMessages.enum.aksi_audit);

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

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

  const [range, setRange] = useState<DayRange>(todayRange);
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [query, setQuery] = useState<AuditCariRequest>(() => dayRangeBounds(todayRange()));

  const [aksi, setAksi] = useState('semua');
  const [pengguna, setPengguna] = useState('');
  const [penggunaError, setPenggunaError] = useState<string | null>(null);
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

  function changeRange(next: DayRange | null) {
    if (!next) return;
    // A picked end before the start never reaches the request; the field says why.
    if (next.end.compare(next.start) < 0) {
      setRangeError(t('common.date.order_error'));
      return;
    }
    setRangeError(null);
    setRange(next);
  }

  function submit(event: React.FormEvent) {
    event.preventDefault();
    // `pengguna_id` is a UUID in the admin contract; free text is never sent.
    const userId = pengguna.trim();
    const userInvalid = userId !== '' && !UUID_RE.test(userId);
    setPenggunaError(userInvalid ? t('audit.filter.user_error') : null);
    if (userInvalid || rangeError) return;
    setQuery({
      ...dayRangeBounds(range),
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

      {/* Frame S13: Pengguna, Aksi and Rentang waktu share one row inside the table card (spec §6 L5). */}
      <Card variant="container" className="flex flex-col overflow-hidden">
        <form className="p-4 large:p-6" onSubmit={submit}>
          <FilterBar>
            <TextField
              label={t('audit.filter.user')}
              value={pengguna}
              autoComplete="off"
              isInvalid={penggunaError !== null}
              errorMessage={penggunaError}
              className="expanded:w-72"
              onChange={(value) => {
                setPengguna(value);
                setPenggunaError(null);
              }}
            />
            <Select
              label={t('audit.filter.action')}
              selectedKey={aksi}
              onSelectionChange={(key) => {
                if (key !== null) setAksi(String(key));
              }}
              className="expanded:w-52 [&>button]:w-full"
            >
              <SelectItem id="semua">{t('common.action.all')}</SelectItem>
              {AKSI_VALUES.map((value) => (
                <SelectItem key={value} id={value}>
                  {enumLabel(i18n, 'aksi_audit', value)}
                </SelectItem>
              ))}
            </Select>
            <div className="flex flex-col gap-1 expanded:w-[280px]">
              <DateRangePicker
                label={t('audit.filter.range')}
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
            <Button type="submit">{t('common.action.search')}</Button>
          </FilterBar>
        </form>

        {audit.status === 'loading' ? (
          <div className="flex flex-col gap-2 px-4 pb-4 large:px-6 large:pb-6">
            <p className="sr-only">{t('common.state.loading')}</p>
            {[0, 1, 2, 3].map((i) => (
              <Skeleton key={i} className="h-10 w-full rounded-extra-small" />
            ))}
          </div>
        ) : null}

        {audit.status === 'error' ? (
          <div className="px-4 pb-4 large:px-6 large:pb-6">
            <QueryError error={audit.error} onRetry={audit.reload} />
          </div>
        ) : null}

        {audit.status === 'success' && rows.length === 0 ? (
          <EmptyState title={t('audit.empty.title')} body={t('audit.empty.body')} />
        ) : null}

        {rows.length > 0 ? (
          <div className="border-t border-outline-variant">
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
                        <td
                          data-label={t('audit.col.count')}
                          className={`${TD} expanded:text-right`}
                        >
                          {PLACEHOLDER}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </ScrollArea>
          </div>
        ) : null}
      </Card>

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
