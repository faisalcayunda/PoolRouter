import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  api,
  type ApiClient,
  type DownstreamPool,
  type HasilK20,
  type Provider,
  type UpstreamPool,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useFormatter } from '../../i18n/useFormatter';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { LinkButton } from '../provider/LinkButton';
import { NonaktifkanDialog } from './NonaktifkanDialog';

export interface DownstreamListScreenProps {
  client?: ApiClient;
}

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

/**
 * S07 daftar downstream pool (ARC-102, ARC-247). Hanya platform admin. Kolom upstream menampilkan
 * nama provider dari upstream pool yang dilayani; bila pencarian nama gagal, id pool yang tampil.
 * Kolom Mode belum punya data di kontrak, jadi tampil sebagai placeholder seperti frame. Alokasi
 * tidak ada di frame daftar; nilainya dibaca dan disunting di form.
 */
export function DownstreamListScreen({ client = api }: DownstreamListScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const fmt = useFormatter();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.downstream'));
  const allowed = role === null || role === 'platform_admin';
  const [dialog, setDialog] = useState<DownstreamPool | null>(null);
  const [failure, setFailure] = useState<unknown>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const pools = useQuery<DownstreamPool[]>(
    (signal) => client.listDownstreamPools({ signal }),
    'downstream',
    { enabled: allowed },
  );
  const upstreams = useQuery<UpstreamPool[]>(
    (signal) => client.upstreamPools({ signal }),
    'upstream',
    { enabled: allowed },
  );
  const providers = useQuery<Provider[]>(
    (signal) => client.listProviders({ signal }),
    'providers',
    {
      enabled: allowed,
    },
  );

  if (!allowed) return <Forbidden />;

  const rows = pools.status === 'success' ? pools.data : [];
  const providerNames = new Map(
    (providers.status === 'success' ? providers.data : []).map((p) => [p.provider_id, p.nama]),
  );
  const upstreamNames = new Map(
    (upstreams.status === 'success' ? upstreams.data : []).map((u) => [
      u.upstream_pool_id,
      providerNames.get(u.provider_id) ?? u.upstream_pool_id,
    ]),
  );
  const upstreamOf = (pool: DownstreamPool) =>
    pool.upstream_pool_ids.map((id) => upstreamNames.get(id) ?? id).join(', ');

  async function aktifkan(pool: DownstreamPool) {
    setBusy(pool.downstream_pool_id);
    setFailure(null);
    try {
      await client.aktifkanDownstreamPool(pool.downstream_pool_id);
      snackbar.notify(t('downstream.activate.done', { nama: pool.nama }));
      pools.reload();
    } catch (error) {
      setFailure(error);
    } finally {
      setBusy(null);
    }
  }

  function selesai(pool: DownstreamPool, hasil: HasilK20) {
    setDialog(null);
    snackbar.notify(
      t('downstream.deactivate.done_counts', {
        nama: pool.nama,
        dibatalkan: hasil.jumlah_cancelled,
        gagal: hasil.jumlah_failed,
      }),
    );
    pools.reload();
  }

  const tambah = <LinkButton href="/downstream/baru">{t('downstream.page.add')}</LinkButton>;

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={t('downstream.page.title')}
        subtitle={t('downstream.page.subtitle')}
        titleRef={titleRef}
        actions={rows.length > 0 ? tambah : null}
      />

      {pools.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {pools.status === 'error' ? <QueryError error={pools.error} onRetry={pools.reload} /> : null}
      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}

      {pools.status === 'success' && rows.length === 0 ? (
        <Card variant="container" className="flex flex-col items-center gap-3 p-8 text-center">
          <h2 className="typescale-title-large text-on-surface">{t('downstream.empty.title')}</h2>
          <p className="typescale-body-medium max-w-xl text-on-surface-variant">
            {t('downstream.empty.body')}
          </p>
          {tambah}
        </Card>
      ) : null}

      {rows.length > 0 ? (
        <Card variant="container" className="overflow-hidden">
          <ScrollArea label={t('downstream.page.title')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('downstream.page.title')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('downstream.col.name')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('downstream.col.mode')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('downstream.col.upstream')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('downstream.col.ttl')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.status')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('upstream.col.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr
                      key={row.downstream_pool_id}
                      className="border-b border-outline-variant text-on-surface"
                    >
                      <td data-label={t('downstream.col.name')} className={TD}>
                        {row.nama}
                      </td>
                      <td data-label={t('downstream.col.mode')} className={TD}>
                        {'–'}
                      </td>
                      <td data-label={t('downstream.col.upstream')} className={TD}>
                        {upstreamOf(row)}
                      </td>
                      <td data-label={t('downstream.col.ttl')} className={TD}>
                        {t('downstream.list.ttl_hours', {
                          jam: fmt.angka(row.ttl_antrean_detik / 3600),
                        })}
                      </td>
                      <td data-label={t('client.col.status')} className={TD}>
                        <StatusLabel tone={row.aktif ? 'success' : 'neutral'}>
                          {row.aktif ? t('common.state.active') : t('upstream.status.inactive')}
                        </StatusLabel>
                      </td>
                      <td data-label={t('upstream.col.actions')} className={TD}>
                        <div className="flex flex-wrap gap-1">
                          <LinkButton
                            href={`/downstream/${encodeURIComponent(row.downstream_pool_id)}`}
                            variant="text"
                            aria-label={t('downstream.list.edit_aria', { nama: row.nama })}
                          >
                            {t('common.action.open')}
                          </LinkButton>
                          {row.aktif ? (
                            <Button
                              variant="text"
                              onPress={() => setDialog(row)}
                              aria-label={t('downstream.list.deactivate_aria', { nama: row.nama })}
                            >
                              {t('upstream.page.deactivate_row')}
                            </Button>
                          ) : (
                            <Button
                              variant="text"
                              isDisabled={busy === row.downstream_pool_id}
                              onPress={() => void aktifkan(row)}
                              aria-label={t('downstream.list.activate_aria', { nama: row.nama })}
                            >
                              {t('upstream.list.activate')}
                            </Button>
                          )}
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        </Card>
      ) : null}

      {dialog ? (
        <NonaktifkanDialog
          title={t('downstream.deactivate.title')}
          tujuan={upstreamOf(dialog)}
          loadPreview={(signal) =>
            client.pratinjauNonaktifDownstreamPool(dialog.downstream_pool_id, { signal })
          }
          onConfirm={(alasan) =>
            client.nonaktifkanDownstreamPool(dialog.downstream_pool_id, alasan)
          }
          onClose={() => setDialog(null)}
          onDone={(hasil) => selesai(dialog, hasil)}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
