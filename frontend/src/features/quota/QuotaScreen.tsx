import { useState, type ReactNode } from 'react';
import { Link } from 'react-aria-components';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type KuotaTracker, type UpstreamPool } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Icon, icons, type IconComponent } from '../../icons';
import { MetricCard } from '../common/MetricCard';
import { LinearProgress } from '../../components/m3/LinearProgress';
import { Select, SelectItem } from '../../components/m3/Select';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useFormatter } from '../../i18n/useFormatter';
import type { MessageId } from '../../i18n/types';
import { FilterBar, SELECT_FILL } from '../common/layout';
import { PageHeader, EmptyState, LoadingState, QueryError } from '../common/page';
import { useQuery } from '../common/useQuery';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';

type Level = 'ok' | 'low' | 'out';

/**
 * Sync-share level thresholds. The number is always printed beside the bar — colour never carries
 * the status alone (section 8, "Kuota: persen di teks, bukan hanya warna").
 */
const HIGH_WATERMARK = 0.8;

function levelOf(used: number, limit: number): Level {
  if (!limit) return 'ok';
  const fraction = used / limit;
  if (fraction >= 1) return 'out';
  return fraction >= HIGH_WATERMARK ? 'low' : 'ok';
}

const TONE: Record<Level, 'success' | 'warning' | 'error'> = {
  ok: 'success',
  low: 'warning',
  out: 'error',
};

/**
 * The tracker reports a 0–1 ratio; `LinearProgress` is an M3/ARIA progress bar, whose value scale
 * is 0–100. Passing the ratio straight through painted an empty bar next to "Terpakai 33%".
 * Clamped so an over-quota row (sisa above kuota_efektif) never reports more than 100%.
 */
function toPercent(fraction: number): number {
  return Math.min(100, Math.max(0, fraction * 100));
}

/** Kartu isi (spec ARC-363 §3): container, radius 16, padding 24. */
const CARD = 'flex flex-col gap-3 p-6';

/** Kartu metrik: ikon + judul, angka besar + "dari N", bilah 8 px, lalu satu baris keterangan. */
function QuotaMetricCard({
  icon,
  label,
  used,
  total,
  caption,
}: {
  icon: IconComponent;
  label: string;
  used: number;
  total: number;
  caption: string;
}) {
  const { t } = useTranslation();
  const format = useFormatter();
  return (
    <MetricCard
      icon={icon}
      title={label}
      titleAs="h3"
      value={format.angka(used)}
      valueNote={t('quota.page.limit_of', { total })}
    >
      <LinearProgress size="large" value={toPercent(total ? used / total : 0)} aria-label={label} />
      <p className="typescale-body-medium text-on-surface-variant">{caption}</p>
    </MetricCard>
  );
}

function PoolQuota({ row }: { row: KuotaTracker }) {
  const { t } = useTranslation();
  const format = useFormatter();
  const used = row.kuota_efektif - row.sisa_periode;
  const usedFraction = row.kuota_efektif ? used / row.kuota_efektif : 0;
  const syncLimit = row.porsi_sync_maks;
  const syncFraction = syncLimit ? row.terpakai_sync_jam_berjalan / syncLimit : 0;
  const level = levelOf(row.terpakai_sync_jam_berjalan, syncLimit);
  const hourLeft = Math.max(0, row.rate_limit_per_jam - row.terpakai_jam_berjalan);
  const hourFraction = row.rate_limit_per_jam
    ? row.terpakai_jam_berjalan / row.rate_limit_per_jam
    : 0;
  // The current hour is the one the row reports (`periode_mulai` is its start, WIB).
  const startOfHour = new Date(row.periode_mulai);
  const endOfHour = new Date(startOfHour.getTime() + 3600_000);

  return (
    <div className="flex flex-col gap-6">
      <div className="grid grid-cols-1 gap-6 expanded:grid-cols-2">
        <QuotaMetricCard
          icon={icons.dataUsage}
          label={t('quota.page.today')}
          used={used}
          total={row.kuota_efektif}
          caption={`${t('quota.page.used_left', {
            persen: Math.round(usedFraction * 100),
            sisa: row.sisa_periode,
          })} · ${t('quota.page.reset_at', { waktu: format.jam(row.periode_selesai) })}`}
        />
        <QuotaMetricCard
          icon={icons.refresh}
          label={t('quota.page.hour', { rentang: format.hourRange(startOfHour, endOfHour) })}
          used={row.terpakai_jam_berjalan}
          total={row.rate_limit_per_jam}
          caption={t('quota.page.used_left', {
            persen: Math.round(hourFraction * 100),
            sisa: hourLeft,
          })}
        />
      </div>

      <div className="grid grid-cols-1 items-stretch gap-6 expanded:grid-cols-2">
        <Card variant="container" className={CARD}>
          <div className="flex items-center justify-between gap-2">
            <h3 className="typescale-title-medium text-on-surface">
              {t('quota.page.sync_heading')}
            </h3>
            <StatusLabel tone={TONE[level]} icon={null}>
              {t(`quota.level.${level}` as MessageId)}
            </StatusLabel>
          </div>
          <p className="typescale-body-medium text-on-surface-variant">
            {t('quota.page.sync_used', {
              terpakai: row.terpakai_sync_jam_berjalan,
              persen: Math.round(syncFraction * 100),
            })}
          </p>
          <LinearProgress
            size="large"
            value={toPercent(syncFraction)}
            aria-label={t('quota.page.sync_heading')}
          />
          <p className="typescale-body-medium text-on-surface-variant">{t('quota.page.note')}</p>
        </Card>

        {/* Riwayat 7 hari belum ada di API (ADR-0008 2.6): placeholder seperti frame Penpot. */}
        <Card variant="container" className={CARD}>
          <div className="flex items-center justify-between gap-2">
            <h3 className="typescale-title-medium text-on-surface">
              {t('quota.page.trend_heading')}
            </h3>
            <span className="typescale-body-medium text-on-surface-variant">
              {t('quota.page.trend_unit')}
            </span>
          </div>
          <div className="flex h-40 flex-col items-center justify-center gap-1 rounded-medium border border-dashed border-outline bg-surface-container-low px-6 text-center expanded:h-[200px]">
            <p className="typescale-title-small text-on-surface">
              {t('quota.page.trend_pending_title')}
            </p>
            <p className="typescale-body-medium max-w-[420px] text-on-surface-variant">
              {t('quota.page.trend_pending_body')}
            </p>
          </div>
        </Card>
      </div>
    </div>
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
  const format = useFormatter();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  useDocumentTitle(t('title.page.quota'));

  const [poolId, setPoolId] = useState<string>('semua');

  const pools = useQuery<UpstreamPool[]>((signal) => client.upstreamPools({ signal }), 'pools');
  const quota = useQuery<KuotaTracker[]>(
    (signal) => client.kuota(poolId === 'semua' ? undefined : poolId, { signal }),
    `quota:${poolId}`,
  );

  /**
   * Section 7: a failed reload keeps the last good numbers on screen with the timestamp they were
   * read at. `useQuery` clears `data` on error, so the last successful result is remembered here.
   */
  const [snapshot, setSnapshot] = useState<{ rows: KuotaTracker[]; at: string } | null>(null);
  const liveRows = quota.status === 'success' ? quota.data : [];
  // Adjusting state while rendering (React's documented pattern) avoids an effect that would flash
  // the error state for one frame before the snapshot is restored.
  if (quota.status === 'success' && liveRows !== snapshot?.rows) {
    setSnapshot({ rows: liveRows, at: new Date().toISOString() });
  }
  const staleSince = quota.status === 'error' && snapshot ? snapshot.at : null;
  const rows = liveRows.length > 0 ? liveRows : (snapshot?.rows ?? []);
  const poolOptions = pools.status === 'success' ? pools.data : [];

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={t('quota.page.title')}
        subtitle={
          rows.length === 1
            ? t('quota.page.meta', {
                nama_pool: t('quota.page.pool_unknown'),
                waktu: format.tanggalWaktu(rows[0]!.dihitung_pada),
              })
            : undefined
        }
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
            {t('quota.page.reload')}
          </Button>
        }
      />

      {/* Spec §6: pemilih pool di baris filter dalam kartu, bukan melayang di bawah judul. */}
      <Card variant="container" className="p-4 large:p-6">
        <FilterBar>
          <Select
            className={`expanded:w-60 ${SELECT_FILL}`}
            label={t('quota.page.pool_filter')}
            selectedKey={poolId}
            onSelectionChange={(key) => {
              if (key !== null) setPoolId(String(key));
            }}
          >
            <SelectItem id="semua">{t('common.action.all')}</SelectItem>
            {/* The pool contract has no display name yet (F0), so each option carries its short
                id; the card heading uses the catalog's generic label instead. */}
            {poolOptions.map((pool) => (
              <SelectItem key={pool.upstream_pool_id} id={pool.upstream_pool_id}>
                {pool.upstream_pool_id.slice(0, 8)}
              </SelectItem>
            ))}
          </Select>
        </FilterBar>
      </Card>

      {quota.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {quota.status === 'error' ? <QueryError error={quota.error} onRetry={quota.reload} /> : null}

      {/* Stale data after a failed reload: the numbers stay, with the time they were last read. */}
      {quota.status === 'error' && staleSince !== null && rows.length > 0 ? (
        <p role="status" className="typescale-body-medium text-on-surface-variant">
          {t('quota.page.stale', { waktu: format.tanggalWaktu(staleSince) })}
        </p>
      ) : null}

      {rows.length > 0 ? (
        <div className="flex flex-col gap-8">
          {rows.map((row) => (
            <div
              key={`${row.upstream_pool_id}-${row.periode_mulai}`}
              className="flex flex-col gap-4"
            >
              {rows.length > 1 ? (
                <h2 className="typescale-title-large text-primary-text">
                  {t('quota.page.meta', {
                    nama_pool: t('quota.page.pool_unknown'),
                    waktu: format.tanggalWaktu(row.dihitung_pada),
                  })}
                </h2>
              ) : null}
              <PoolQuota row={row} />
            </div>
          ))}
        </div>
      ) : quota.status === 'success' ? (
        <div className="flex flex-col gap-2">
          <EmptyState title={t('upstream.empty.title')} body={t('upstream.empty.body')} />
          {/* Section 7 empty state: point at the screen that creates the missing pool (S06). */}
          <div>
            <Link
              href="/upstream"
              className="m3-focus typescale-label-large rounded-small text-primary-text underline-offset-2 hover:underline"
            >
              {t('quota.page.empty_action')}
            </Link>
          </div>
        </div>
      ) : null}
    </section>
  );
}
