import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type KuotaTracker, type UpstreamPool } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Icon, icons } from '../../icons';
import { LinearProgress } from '../../components/m3/LinearProgress';
import { Select, SelectItem } from '../../components/m3/Select';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useFormatter } from '../../i18n/useFormatter';
import type { MessageId } from '../../i18n/types';
import { PageHeader, EmptyState, LoadingState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';

type Level = 'ok' | 'low' | 'out';

/**
 * Quota level thresholds. The number is always printed beside the bar — colour never carries the
 * status alone (section 8, "Kuota: persen di teks, bukan hanya warna").
 */
const LOW_WATERMARK = 0.2;

function levelOf(kuotaEfektif: number, sisa: number): Level {
  if (sisa <= 0) return 'out';
  return sisa / Math.max(1, kuotaEfektif) <= LOW_WATERMARK ? 'low' : 'ok';
}

const TONE: Record<Level, 'success' | 'warning' | 'error'> = {
  ok: 'success',
  low: 'warning',
  out: 'error',
};

function PoolQuotaCard({ row }: { row: KuotaTracker }) {
  const { t } = useTranslation();
  const format = useFormatter();
  const level = levelOf(row.kuota_efektif, row.sisa_periode);
  const usedFraction = row.kuota_efektif
    ? (row.kuota_efektif - row.sisa_periode) / row.kuota_efektif
    : 0;
  const syncLimit = row.porsi_sync_maks;
  // The current hour is the one the row reports (`periode_mulai` is its start, WIB).
  const startOfHour = new Date(row.periode_mulai);
  const endOfHour = new Date(startOfHour.getTime() + 3600_000);

  return (
    <Card variant="outlined" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="flex flex-col">
          <h2 className="typescale-title-medium">{row.upstream_pool_id.slice(0, 8)}</h2>
          <p className="typescale-body-small text-on-surface-variant">
            {t('quota.page.meta', {
              nama_pool: row.upstream_pool_id.slice(0, 8),
              waktu: format.tanggalWaktu(row.dihitung_pada),
            })}
          </p>
        </div>
        <StatusLabel tone={TONE[level]}>{t(`quota.level.${level}` as MessageId)}</StatusLabel>
      </div>

      <div className="flex flex-col gap-1">
        <p className="typescale-label-large">{t('quota.page.today')}</p>
        <p className="typescale-body-medium">
          {t('quota.page.used_left', {
            persen: Math.round(usedFraction * 100),
            sisa: row.sisa_periode,
          })}
        </p>
        <LinearProgress value={usedFraction} aria-label={t('quota.page.today')} />
        <p className="typescale-body-small text-on-surface-variant">
          {t('quota.page.reset_at', {
            waktu: format.jam(row.periode_selesai),
          })}
        </p>
      </div>

      <div className="flex flex-col gap-1">
        <p className="typescale-label-large">{t('quota.page.sync_heading')}</p>
        <p className="typescale-body-medium">
          {t('quota.page.sync_used', {
            terpakai: row.terpakai_sync_jam_berjalan,
            persen: syncLimit ? Math.round((row.terpakai_sync_jam_berjalan / syncLimit) * 100) : 0,
          })}
        </p>
        <LinearProgress
          value={syncLimit ? row.terpakai_sync_jam_berjalan / syncLimit : 0}
          aria-label={t('quota.page.sync_heading')}
        />
      </div>

      <div className="flex flex-col gap-1">
        <p className="typescale-label-large">
          {t('quota.page.hour', { rentang: format.hourRange(startOfHour, endOfHour) })}
        </p>
        <p className="typescale-body-medium">
          {t('quota.page.of_total', {
            terpakai: row.terpakai_jam_berjalan,
            total: row.rate_limit_per_jam,
          })}
        </p>
        <LinearProgress
          value={row.rate_limit_per_jam ? row.terpakai_jam_berjalan / row.rate_limit_per_jam : 0}
          aria-label={t('quota.page.hour', { rentang: format.hourRange(startOfHour, endOfHour) })}
        />
      </div>
    </Card>
  );
}

export interface QuotaScreenProps {
  client?: ApiClient;
}

/**
 * Quota tracker (US-12, KP-07). Read-only and computed from the database, not Redis; the screen
 * therefore says so in the footnote and never offers a refresh interval of its own beyond the
 * explicit "Muat ulang".
 */
export function QuotaScreen({ client = api }: QuotaScreenProps): ReactNode {
  const { t } = useTranslation();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.quota'));

  const [poolId, setPoolId] = useState<string>('semua');

  const pools = useQuery<UpstreamPool[]>((signal) => client.upstreamPools({ signal }), 'pools');
  const quota = useQuery<KuotaTracker[]>(
    (signal) => client.kuota(poolId === 'semua' ? undefined : poolId, { signal }),
    `quota:${poolId}`,
  );

  const rows = quota.status === 'success' ? quota.data : [];
  const poolOptions = pools.status === 'success' ? pools.data : [];

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={t('quota.page.title')}
        subtitle={t('quota.page.note')}
        titleRef={titleRef}
        actions={
          <Button
            variant="outlined"
            onPress={() => {
              pools.reload();
              quota.reload();
            }}
          >
            <Icon icon={icons.refresh} aria-hidden="true" />
            {t('common.action.retry')}
          </Button>
        }
      >
        <div className="flex flex-wrap items-end gap-3">
          <Select
            label={t('quota.page.title')}
            selectedKey={poolId}
            onSelectionChange={(key) => {
              if (key !== null) setPoolId(String(key));
            }}
          >
            <SelectItem id="semua">{t('common.action.all')}</SelectItem>
            {poolOptions.map((pool) => (
              <SelectItem key={pool.upstream_pool_id} id={pool.upstream_pool_id}>
                {pool.upstream_pool_id.slice(0, 8)}
              </SelectItem>
            ))}
          </Select>
        </div>
      </PageHeader>

      {quota.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {quota.status === 'error' ? <QueryError error={quota.error} onRetry={quota.reload} /> : null}

      {quota.status === 'success' ? (
        rows.length === 0 ? (
          <EmptyState title={t('upstream.empty.title')} body={t('upstream.empty.body')} />
        ) : (
          <div className="grid grid-cols-1 gap-3 expanded:grid-cols-2">
            {rows.map((row) => (
              <PoolQuotaCard key={`${row.upstream_pool_id}-${row.periode_mulai}`} row={row} />
            ))}
          </div>
        )
      ) : null}

      <p className="typescale-body-small text-on-surface-variant">{t('quota.page.note')}</p>
    </section>
  );
}
