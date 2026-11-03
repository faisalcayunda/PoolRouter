import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type AuditCariRequest, type AuditAksesPage } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Select, SelectItem } from '../../components/m3/Select';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useFormatter } from '../../i18n/useFormatter';
import { labelOr } from '../../i18n/labelOr';
import { DateRangeField, type DateRange } from '../common/DateRangeField';
import { PageHeader, EmptyState, LoadingState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { ScrollArea } from '../common/ScrollArea';

/** The API rejects ranges over 31 days (section 18.3); the picker stays inside that bound. */
const MAX_RANGE_DAYS = 31;

/** Actions the audit table can name. Anything else shows its raw code rather than a blank cell. */
const ACTIONS = [
  'cari_nik',
  'lihat_payload',
  'lihat_audit',
  'ekspor',
  'ditolak',
  'login',
  'cari_log',
] as const;

function lastWeek(): DateRange {
  const now = new Date();
  const start = new Date(now.getTime() - 7 * 24 * 3600 * 1000);
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

function withinDays(range: DateRange, days: number): boolean {
  const span = (Date.parse(range.to) - Date.parse(range.from)) / 86_400_000;
  return span <= days;
}

export interface AuditScreenProps {
  client?: ApiClient;
}

/**
 * S13 — Audit akses (auditor). Read-only by contract: the screen offers no edit or delete action
 * and the API has no endpoint for one (ADR-0003 rev 3 §3.6, append-only). Rows carry no NIK hash,
 * so nothing here can be re-joined to a person.
 */
export function AuditScreen({ client = api }: AuditScreenProps): ReactNode {
  const { t, i18n: instance } = useTranslation();
  const format = useFormatter();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.audit'));

  const [range, setRange] = useState<DateRange | null>(() => lastWeek());
  const [aksi, setAksi] = useState<string>('semua');
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState<{ body: AuditCariRequest; key: string } | null>(null);

  const query = useQuery<AuditAksesPage>(
    (signal) => client.cariAuditAkses(submitted!.body, { signal }),
    submitted?.key ?? 'idle',
    { enabled: submitted !== null },
  );

  function submit() {
    if (!range || !withinDays(range, MAX_RANGE_DAYS)) {
      // GAP: audit.filter.range_help — no catalog line yet; the range legend states the rule.
      setRangeError(t('log.filter.range'));
      return;
    }
    setRangeError(null);
    const body: AuditCariRequest = {
      dari: range.from,
      sampai: range.to,
      aksi: aksi === 'semua' ? null : aksi,
    };
    setSubmitted({ body, key: JSON.stringify(body) });
  }

  const rows = query.status === 'success' ? query.data.baris : [];

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={t('audit.page.title')}
        subtitle={t('audit.page.subtitle')}
        titleRef={titleRef}
      />

      <Card variant="outlined">
        <form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
          aria-label={t('audit.filter.action')}
        >
          <h2 className="sr-only">{t('audit.filter.action')}</h2>
          <div className="flex flex-wrap items-start gap-3">
            <Select
              label={t('audit.filter.action')}
              selectedKey={aksi}
              onSelectionChange={(key) => {
                if (key !== null) setAksi(String(key));
              }}
            >
              <SelectItem id="semua">{t('common.action.all')}</SelectItem>
              {ACTIONS.map((action) => (
                <SelectItem key={action} id={action}>
                  {labelOr(instance, `enum.aksi_audit.${action}`, action)}
                </SelectItem>
              ))}
            </Select>
            <DateRangeField
              legend={t('log.filter.range')}
              // GAP: audit.filter.range_help — "Paling panjang 31 hari" is not in the catalog yet.
              description={t('log.filter.range')}
              value={range}
              onChange={(next) => {
                setRange(next);
                if (next) setRangeError(null);
              }}
              errorMessage={rangeError}
            />
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <Button variant="filled" type="submit">
              {t('common.action.search')}
            </Button>
            <p className="typescale-body-small text-on-surface-variant">
              {t('audit.page.subtitle')}
            </p>
          </div>
        </form>
      </Card>

      <h2 className="sr-only">{t('audit.page.title')}</h2>

      {query.status === 'error' ? <QueryError error={query.error} onRetry={query.reload} /> : null}
      {query.status === 'loading' && submitted ? (
        <LoadingState label={t('common.state.loading')} />
      ) : null}

      {query.status === 'success' ? (
        rows.length === 0 ? (
          <EmptyState title={t('audit.empty.title')} body={t('audit.empty.body')} />
        ) : (
          <Card variant="filled" className="p-0">
            <ScrollArea label={t('audit.page.title')}>
              <table className="w-full border-collapse">
                <caption className="sr-only">{t('audit.page.title')}</caption>
                <thead>
                  <tr>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('log.col.time')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('audit.col.user')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('audit.col.action')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('audit.col.action')}
                    </th>
                    <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                      {t('nik.reason.title')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={row.audit_id} className="h-10">
                      <td className="typescale-body-medium px-4">
                        {format.tanggalWaktu(row.dilakukan_pada)}
                      </td>
                      <td className="typescale-body-medium px-4 font-mono">{row.pengguna_id}</td>
                      <td className="typescale-body-medium px-4">
                        {labelOr(instance, `enum.aksi_audit.${row.aksi}`, row.aksi)}
                      </td>
                      <td className="typescale-body-medium px-4">
                        <StatusLabel tone={row.hasil === 'berhasil' ? 'success' : 'error'}>
                          {labelOr(instance, `audit.result.${row.hasil}`, row.hasil)}
                        </StatusLabel>
                      </td>
                      <td className="typescale-body-medium px-4">{row.alasan ?? '—'}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </ScrollArea>
          </Card>
        )
      ) : null}
    </section>
  );
}
