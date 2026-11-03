import { useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type AuditAksesPage, type AuditCariRequest } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Skeleton } from '../../components/m3/Skeleton';
import { StatusLabel } from '../../components/m3/StatusLabel';
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

export interface AuditScreenProps {
  client?: ApiClient;
}

/**
 * S13 — Audit akses. Read-only: no edit or delete action exists (section 7).
 *
 * Columns follow what `searchAuditAkses` actually returns: time, user id, action, result and
 * reason. The per-row role, filter and record count are not part of the admin contract in F0, so no
 * column is rendered for them — an empty column reading "—" would be worse than omitting it.
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

  const audit = useQuery<AuditAksesPage>(
    (signal) => client.cariAuditAkses(query, { signal }),
    JSON.stringify(query),
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
    setQuery({ dari: from, sampai: to });
  }

  return (
    <section className="flex w-full flex-col gap-6">
      <PageHeader
        title={t('audit.page.title')}
        subtitle={t('audit.page.subtitle')}
        titleRef={titleRef}
      />

      <form className="flex flex-col gap-4" onSubmit={submit}>
        <DateRangeField
          legend={t('audit.filter.range')}
          description={t('audit.filter.range_help')}
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
                    <th
                      scope="col"
                      className="border-b border-outline-variant px-4 py-3 font-medium"
                    >
                      {t('audit.col.time')}
                    </th>
                    <th
                      scope="col"
                      className="hide-md border-b border-outline-variant px-4 py-3 font-medium"
                    >
                      {t('audit.col.user')}
                    </th>
                    <th
                      scope="col"
                      className="border-b border-outline-variant px-4 py-3 font-medium"
                    >
                      {t('audit.col.action')}
                    </th>
                    <th
                      scope="col"
                      className="border-b border-outline-variant px-4 py-3 font-medium"
                    >
                      {t('audit.col.result')}
                    </th>
                    <th
                      scope="col"
                      className="border-b border-outline-variant px-4 py-3 font-medium"
                    >
                      {t('audit.col.reason')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr
                      key={row.audit_id}
                      className="border-b border-outline-variant text-on-surface"
                    >
                      <td
                        data-label={t('audit.col.time')}
                        className="typescale-body-medium px-4 py-3 align-middle"
                      >
                        {format.tanggalWaktu(row.dilakukan_pada)}
                      </td>
                      <td
                        data-label={t('audit.col.user')}
                        className="hide-md typescale-body-medium px-4 py-3 align-middle"
                      >
                        {/* Section 13.2: the raw id is an identifier, labelled as one — never a name. */}
                        <span className="break-all font-mono text-on-surface-variant">
                          {row.pengguna_id}
                        </span>
                      </td>
                      <td
                        data-label={t('audit.col.action')}
                        className="typescale-body-medium px-4 py-3 align-middle"
                      >
                        {/* `labelOr` keeps an action outside the catalog from rendering as a raw key. */}
                        {enumLabel(i18n, 'aksi_audit', row.aksi)}
                      </td>
                      <td
                        data-label={t('audit.col.result')}
                        className="typescale-body-medium px-4 py-3 align-middle"
                      >
                        <StatusLabel tone={row.hasil === 'berhasil' ? 'success' : 'error'}>
                          {enumLabel(i18n, 'hasil', row.hasil)}
                        </StatusLabel>
                      </td>
                      <td
                        data-label={t('audit.col.reason')}
                        className="typescale-body-medium px-4 py-3 align-middle"
                      >
                        {row.alasan ?? '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        </Card>
      ) : null}

      <p className="typescale-body-small text-on-surface-variant">{t('audit.filter.note')}</p>
    </section>
  );
}
