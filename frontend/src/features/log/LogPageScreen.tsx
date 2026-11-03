import { useState, type ReactNode } from 'react';
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
import { Card } from '../../components/m3/Card';
import { IconButton } from '../../components/m3/IconButton';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { TextField } from '../../components/m3/TextField';
import { Icon, icons } from '../../icons';
import { useFormatter } from '../../i18n/useFormatter';
import type { MessageId } from '../../i18n/types';
import { DateRangeField, type DateRange } from '../common/DateRangeField';
import { PageHeader, EmptyState, LoadingState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { ScrollArea } from '../common/ScrollArea';
import { useRole } from '../common/RoleContext';
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
    startDate: s.date,
    startTime: s.time,
    endDate: e.date,
    endTime: e.time,
    from: `${s.date}T${s.time}:00+07:00`,
    to: `${e.date}T${e.time}:00+07:00`,
  };
}

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
 */
export function LogPageScreen({
  client = api,
  canOpenPayload,
  onOpenPayload,
  initialCode = '',
}: LogPageScreenProps): ReactNode {
  const { t } = useTranslation();
  const format = useFormatter();
  const role = useRole();
  // Section 18.3: only an auditor gets the payload column. An explicit prop still wins so the
  // preview harness and tests can pin the state before the session hook lands.
  const payloadAllowed = canOpenPayload ?? role === 'auditor';
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.log'));

  const [code, setCode] = useState(initialCode);
  const [queue, setQueue] = useState('');
  const [range, setRange] = useState<DateRange | null>(() => lastDay());
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [submitted, setSubmitted] = useState<{ body: LogCariRequest; key: string } | null>(null);
  /** Row whose payload is open (section 18.3); `null` keeps the dialog unmounted. */
  const [payloadFor, setPayloadFor] = useState<LogBaris | null>(null);

  function openPayload(row: LogBaris) {
    onOpenPayload?.(row);
    setPayloadFor(row);
  }

  const query = useQuery<LogPage>(
    (signal) => client.cariLog(submitted!.body, { signal }),
    submitted?.key ?? 'idle',
    { enabled: submitted !== null },
  );

  function submit() {
    if (!range) {
      setRangeError(t('nik.page.range_only_note'));
      return;
    }
    setRangeError(null);
    // NIK is never a filter here; the operator hint points to the auditor screen instead.
    const body: LogCariRequest = {
      dari: range.from,
      sampai: range.to,
      kode_transaksi: code.trim() || null,
      queue_id: queue.trim() || null,
    };
    setSubmitted({ body, key: JSON.stringify(body) });
  }

  const rows = query.status === 'success' ? query.data.baris : [];

  return (
    <section className="flex flex-col gap-4">
      <PageHeader title={t('log.page.title')} subtitle={t('log.page.subtitle')} titleRef={titleRef}>
        <InlineAlert tone="info" live="polite">
          {t('log.page.nik_hint')}
        </InlineAlert>
      </PageHeader>

      <Card variant="outlined">
        <form
          className="flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
          aria-label={t('log.filter.range')}
        >
          <div className="flex flex-wrap items-start gap-3">
            <TextField
              label={t('log.filter.code')}
              description={t('log.filter.code_help')}
              value={code}
              onChange={setCode}
            />
            <TextField
              label={t('log.filter.queue')}
              description={t('log.filter.queue_help')}
              value={queue}
              onChange={setQueue}
            />
          </div>
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

      {query.status === 'error' ? <QueryError error={query.error} onRetry={query.reload} /> : null}

      {query.status === 'loading' && submitted ? (
        <LoadingState label={t('common.state.loading')} />
      ) : null}

      {query.status === 'success' ? (
        rows.length === 0 ? (
          <EmptyState title={t('common.empty.logs.title')} body={t('common.empty.logs.body')} />
        ) : (
          <Card variant="filled" className="p-0">
            <ScrollArea label={t('log.page.title')}>
              <table className="w-full border-collapse">
                <caption className="sr-only">{t('log.page.title')}</caption>
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
                    {payloadAllowed ? (
                      <th scope="col" className="typescale-label-large px-4 py-3 text-left">
                        {t('nik.page.payload')}
                      </th>
                    ) : null}
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
                      {payloadAllowed ? (
                        <td className="px-4">
                          <IconButton
                            aria-label={t('nik.page.payload', { kode: row.kode_transaksi })}
                            onPress={() => openPayload(row)}
                          >
                            <Icon icon={icons.visibility} />
                          </IconButton>
                        </td>
                      ) : null}
                    </tr>
                  ))}
                </tbody>
              </table>
            </ScrollArea>
          </Card>
        )
      ) : null}

      {/* Section 18.3: payload opens only from S11 with a mandatory reason; `no-store`, no copy,
          no download. Closing it drops the payload — reopening needs a fresh reason. */}
      <PayloadDialog
        isOpen={payloadFor !== null}
        kodeTransaksi={payloadFor?.kode_transaksi ?? ''}
        // Section 18.3: every opening needs its own reason — nothing carries over from S11.
        initialReason=""
        client={client}
        onClose={() => setPayloadFor(null)}
      />
    </section>
  );
}
