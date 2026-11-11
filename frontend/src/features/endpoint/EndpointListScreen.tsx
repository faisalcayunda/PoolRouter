import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  api,
  ApiError,
  type ApiClient,
  type DownstreamPool,
  type Endpoint,
  type Provider,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { TextArea } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { LinkButton } from '../provider/LinkButton';
import { ALASAN_MAKS, ALASAN_MIN } from './endpointForm';

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

interface ListData {
  endpoints: Endpoint[];
  providers: Provider[];
  pools: DownstreamPool[];
}

/**
 * Dialog nonaktifkan endpoint (K-20): menampilkan pratinjau dampak dulu, lalu meminta alasan
 * (10..500 karakter). Tombol konfirmasi baru muncul setelah pratinjau terbaca, jadi admin tidak
 * menonaktifkan tanpa melihat berapa request yang terdampak.
 */
function NonaktifDialog({
  client,
  endpoint,
  nama,
  onClose,
  onDone,
}: {
  client: ApiClient;
  endpoint: Endpoint;
  nama: string;
  onClose: () => void;
  onDone: (nama: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [alasan, setAlasan] = useState('');
  const [attempted, setAttempted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const preview = useQuery(
    (signal) => client.pratinjauNonaktifkanEndpoint(endpoint.endpoint_id, { signal }),
    `endpoint-preview:${endpoint.endpoint_id}`,
  );
  const trimmed = alasan.trim();
  const alasanSalah = trimmed.length < ALASAN_MIN || trimmed.length > ALASAN_MAKS;

  async function konfirmasi() {
    if (busy) return;
    setAttempted(true);
    if (alasanSalah) return;
    setBusy(true);
    setFailure(null);
    try {
      await client.nonaktifkanEndpoint(endpoint.endpoint_id, trimmed);
      onDone(nama);
    } catch (error) {
      setFailure(error);
      setBusy(false);
    }
  }

  return (
    <Dialog
      isOpen
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t('endpoint.deactivate.title', { nama })}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onClose}>
            {t('common.action.cancel')}
          </Button>
          {preview.status === 'success' ? (
            <Button destructive isDisabled={busy} onPress={() => void konfirmasi()}>
              {t('endpoint.deactivate.confirm')}
            </Button>
          ) : null}
        </>
      }
    >
      <div className="flex flex-col gap-3 pt-2">
        {preview.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
        {preview.status === 'error' ? (
          <QueryError error={preview.error} onRetry={preview.reload} />
        ) : null}
        {preview.status === 'success' ? (
          <>
            <p>{t('endpoint.deactivate.body')}</p>
            <ul className="list-disc pl-5" aria-label={t('endpoint.deactivate.impact')}>
              <li>{t('endpoint.deactivate.cancelled', { jumlah: preview.data.akan_cancelled })}</li>
              <li>{t('endpoint.deactivate.failed', { jumlah: preview.data.akan_failed })}</li>
            </ul>
            <TextArea
              label={t('endpoint.deactivate.reason.label')}
              description={t('endpoint.form.reason.help', {
                jumlah: trimmed.length,
                batas: ALASAN_MAKS,
              })}
              value={alasan}
              onChange={setAlasan}
              rows={3}
              autoComplete="off"
              isInvalid={attempted && alasanSalah}
              errorMessage={t('endpoint.form.error.alasan_rentang')}
            />
          </>
        ) : null}
        {failure !== null ? (
          <InlineAlert tone="error">
            {failure instanceof ApiError && failure.status === 422
              ? t('endpoint.form.error.alasan_rentang')
              : errorMessage(t, i18n, failure).message}
          </InlineAlert>
        ) : null}
      </div>
    </Dialog>
  );
}

export interface EndpointListScreenProps {
  client?: ApiClient;
}

/** S04b daftar endpoint. Hanya platform admin; API tetap penjaga terakhir untuk peran lain. */
export function EndpointListScreen({ client = api }: EndpointListScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.endpoint'));
  const allowed = role === null || role === 'platform_admin';
  const [target, setTarget] = useState<Endpoint | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [failure, setFailure] = useState<unknown>(null);

  const data = useQuery<ListData>(
    async (signal) => {
      const [endpoints, providers, pools] = await Promise.all([
        client.listEndpoints({ signal }),
        client.listProviders({ signal }),
        client.listDownstreamPools({ signal }),
      ]);
      return { endpoints, providers, pools };
    },
    'endpoints',
    { enabled: allowed },
  );

  if (!allowed) return <Forbidden />;

  const rows = data.status === 'success' ? data.data.endpoints : [];
  const providerName = (id: string) =>
    data.status === 'success'
      ? (data.data.providers.find((p) => p.provider_id === id)?.nama ?? id)
      : id;
  const poolName = (id: string) =>
    data.status === 'success'
      ? (data.data.pools.find((p) => p.downstream_pool_id === id)?.nama ?? id)
      : id;

  async function aktifkan(row: Endpoint) {
    if (busyId) return;
    setBusyId(row.endpoint_id);
    setFailure(null);
    try {
      await client.aktifkanEndpoint(row.endpoint_id);
      snackbar.notify(t('endpoint.activate.done', { nama: providerName(row.provider_id) }));
      data.reload();
    } catch (error) {
      setFailure(error);
    } finally {
      setBusyId(null);
    }
  }

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={t('endpoint.page.title')}
        subtitle={t('endpoint.page.subtitle')}
        titleRef={titleRef}
        actions={
          rows.length > 0 ? (
            <LinkButton href="/endpoint/baru">{t('endpoint.page.add')}</LinkButton>
          ) : null
        }
      />

      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}
      {data.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {data.status === 'error' ? <QueryError error={data.error} onRetry={data.reload} /> : null}

      {data.status === 'success' && rows.length === 0 ? (
        <Card variant="container" className="flex flex-col items-center gap-3 p-8 text-center">
          <h2 className="typescale-title-large text-on-surface">{t('endpoint.empty.title')}</h2>
          <p className="typescale-body-medium max-w-xl text-on-surface-variant">
            {t('endpoint.empty.body')}
          </p>
          <LinkButton href="/endpoint/baru">{t('endpoint.page.add')}</LinkButton>
        </Card>
      ) : null}

      {rows.length > 0 ? (
        <Card variant="container" className="overflow-hidden">
          <ScrollArea label={t('endpoint.page.title')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('endpoint.page.title')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('endpoint.list.col.provider')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('endpoint.list.col.pool')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('endpoint.list.col.version')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('endpoint.list.col.metadata')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('endpoint.list.col.status')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('endpoint.list.col.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => {
                    const nama = providerName(row.provider_id);
                    return (
                      <tr
                        key={row.endpoint_id}
                        className="border-b border-outline-variant text-on-surface"
                      >
                        <td data-label={t('endpoint.list.col.provider')} className={TD}>
                          {nama}
                        </td>
                        <td data-label={t('endpoint.list.col.pool')} className={TD}>
                          {poolName(row.downstream_pool_id)}
                        </td>
                        <td data-label={t('endpoint.list.col.version')} className={TD}>
                          {row.endpoint_config_version}
                        </td>
                        <td data-label={t('endpoint.list.col.metadata')} className={TD}>
                          {t('endpoint.list.metadata_count', { count: row.metadata_fields.length })}
                        </td>
                        <td data-label={t('endpoint.list.col.status')} className={TD}>
                          <StatusLabel tone={row.aktif ? 'success' : 'neutral'}>
                            {row.aktif ? t('common.state.active') : t('endpoint.list.inactive')}
                          </StatusLabel>
                        </td>
                        <td data-label={t('endpoint.list.col.actions')} className={TD}>
                          <div className="flex flex-wrap items-center gap-1">
                            <LinkButton
                              href={`/endpoint/${encodeURIComponent(row.endpoint_id)}`}
                              variant="text"
                              aria-label={t('endpoint.list.edit_aria', { nama })}
                            >
                              {t('common.action.edit')}
                            </LinkButton>
                            {row.aktif ? (
                              <Button
                                variant="text"
                                destructive
                                aria-label={t('endpoint.list.deactivate_aria', { nama })}
                                onPress={() => setTarget(row)}
                              >
                                {t('endpoint.list.deactivate')}
                              </Button>
                            ) : (
                              <Button
                                variant="text"
                                isDisabled={busyId === row.endpoint_id}
                                aria-label={t('endpoint.list.activate_aria', { nama })}
                                onPress={() => void aktifkan(row)}
                              >
                                {t('endpoint.list.activate')}
                              </Button>
                            )}
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        </Card>
      ) : null}

      {target ? (
        <NonaktifDialog
          client={client}
          endpoint={target}
          nama={providerName(target.provider_id)}
          onClose={() => setTarget(null)}
          onDone={(nama) => {
            setTarget(null);
            snackbar.notify(t('endpoint.deactivate.done', { nama }));
            data.reload();
          }}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
