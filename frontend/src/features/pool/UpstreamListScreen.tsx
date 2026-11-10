import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  api,
  type ApiClient,
  type HasilK20,
  type Provider,
  type UpstreamPool,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { LinkButton } from '../provider/LinkButton';
import { NonaktifkanDialog } from './NonaktifkanDialog';

export interface UpstreamListScreenProps {
  client?: ApiClient;
}

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

/**
 * S06 daftar upstream pool (ARC-102, ARC-247). Hanya platform admin. Entitas upstream pool belum
 * punya nama sendiri di kontrak: kolom Nama tampil sebagai placeholder seperti frame, dan aksi
 * baris memakai nama providernya; bila daftar provider gagal dibaca, jatuh ke id pool.
 */
export function UpstreamListScreen({ client = api }: UpstreamListScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.upstream'));
  const allowed = role === null || role === 'platform_admin';
  const [dialog, setDialog] = useState<UpstreamPool | null>(null);
  const [failure, setFailure] = useState<unknown>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const pools = useQuery<UpstreamPool[]>((signal) => client.upstreamPools({ signal }), 'upstream', {
    enabled: allowed,
  });
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
  const nameOf = (pool: UpstreamPool) =>
    providerNames.get(pool.provider_id) ?? pool.upstream_pool_id;

  async function aktifkan(pool: UpstreamPool) {
    setBusy(pool.upstream_pool_id);
    setFailure(null);
    try {
      await client.aktifkanUpstreamPool(pool.upstream_pool_id);
      snackbar.notify(t('upstream.activate.done', { nama: nameOf(pool) }));
      pools.reload();
    } catch (error) {
      setFailure(error);
    } finally {
      setBusy(null);
    }
  }

  function selesai(pool: UpstreamPool, hasil: HasilK20) {
    setDialog(null);
    snackbar.notify(
      t('upstream.deactivate.done', {
        nama: nameOf(pool),
        dibatalkan: hasil.jumlah_cancelled,
        gagal: hasil.jumlah_failed,
      }),
    );
    pools.reload();
  }

  const tambah = <LinkButton href="/upstream/baru">{t('upstream.page.add')}</LinkButton>;

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={t('upstream.page.title')}
        subtitle={t('upstream.page.subtitle')}
        titleRef={titleRef}
        actions={rows.length > 0 ? tambah : null}
      />

      {pools.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {pools.status === 'error' ? <QueryError error={pools.error} onRetry={pools.reload} /> : null}
      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}

      {pools.status === 'success' && rows.length === 0 ? (
        <Card variant="outlined" className="flex flex-col items-center gap-3 p-8 text-center">
          <h2 className="typescale-title-large text-on-surface">{t('upstream.empty.title')}</h2>
          <p className="typescale-body-medium max-w-xl text-on-surface-variant">
            {t('upstream.empty.body')}
          </p>
          {tambah}
        </Card>
      ) : null}

      {rows.length > 0 ? (
        <Card variant="outlined" className="p-0">
          <ScrollArea label={t('upstream.page.title')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('upstream.page.title')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('upstream.col.name')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('upstream.col.provider')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('upstream.col.quota')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('upstream.col.status')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('upstream.col.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr
                      key={row.upstream_pool_id}
                      className="border-b border-outline-variant text-on-surface"
                    >
                      <td data-label={t('upstream.col.name')} className={TD}>
                        {'–'}
                      </td>
                      <td data-label={t('upstream.col.provider')} className={TD}>
                        {providerNames.get(row.provider_id) ?? row.provider_id}
                      </td>
                      <td data-label={t('upstream.col.quota')} className={TD}>
                        {t('upstream.page.quota_per_day', { jumlah: row.kuota_harian })}
                      </td>
                      <td data-label={t('upstream.col.status')} className={TD}>
                        <StatusLabel tone={row.aktif ? 'success' : 'neutral'}>
                          {row.aktif ? t('common.state.active') : t('upstream.status.inactive')}
                        </StatusLabel>
                      </td>
                      <td data-label={t('upstream.col.actions')} className={TD}>
                        <div className="flex flex-wrap gap-1">
                          <LinkButton
                            href={`/upstream/${encodeURIComponent(row.upstream_pool_id)}`}
                            variant="text"
                            aria-label={t('upstream.list.edit_aria', { nama: nameOf(row) })}
                          >
                            {t('common.action.edit')}
                          </LinkButton>
                          {row.aktif ? (
                            <Button
                              variant="text"
                              onPress={() => setDialog(row)}
                              aria-label={t('upstream.list.deactivate_aria', { nama: nameOf(row) })}
                            >
                              {t('upstream.page.deactivate_row')}
                            </Button>
                          ) : (
                            <Button
                              variant="text"
                              isDisabled={busy === row.upstream_pool_id}
                              onPress={() => void aktifkan(row)}
                              aria-label={t('upstream.list.activate_aria', { nama: nameOf(row) })}
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
          title={t('upstream.deactivate.title')}
          tujuan={providerNames.get(dialog.provider_id) ?? dialog.provider_id}
          loadPreview={(signal) =>
            client.pratinjauNonaktifUpstreamPool(dialog.upstream_pool_id, { signal })
          }
          onConfirm={(alasan) => client.nonaktifkanUpstreamPool(dialog.upstream_pool_id, alasan)}
          onClose={() => setDialog(null)}
          onDone={(hasil) => selesai(dialog, hasil)}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
