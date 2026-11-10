import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  api,
  ApiError,
  type ApiClient,
  type Client,
  type WebhookConfig,
  type WebhookLog,
  type WebhookSecret,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { Switch } from '../../components/m3/Switch';
import { TextField } from '../../components/m3/TextField';
import { useFormatter } from '../../i18n/useFormatter';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { canViewClients } from './access';
import { ClientSubnav } from './ClientSubnav';
import { OnceSecretDialog } from './OnceSecretDialog';

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';
const HARI_MS = 24 * 60 * 60 * 1000;

/** Alamat webhook wajib https (server memeriksa lagi dan mencocokkan allowlist domain). */
export const ALAMAT_HTTPS = /^https:\/\/[^\s/?#]+\S*$/i;

type Hasil = 'semua' | 'terkirim' | 'gagal';

function ConfigForm({
  client,
  clientId,
  config,
  onSaved,
}: {
  client: ApiClient;
  clientId: string;
  config: WebhookConfig | null;
  onSaved: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [url, setUrl] = useState(config?.webhook_url ?? '');
  const [aktif, setAktif] = useState(config?.aktif ?? true);
  const [attempted, setAttempted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const urlError = ALAMAT_HTTPS.test(url.trim()) ? null : t('webhook.address.error');

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    setAttempted(true);
    if (saving || urlError) return;
    setSaving(true);
    setFailure(null);
    try {
      await client.putWebhookConfig(clientId, { webhook_url: url.trim(), aktif });
      onSaved();
    } catch (error) {
      setFailure(error);
    }
    setSaving(false);
  }

  // 422 dari server berarti alamat ditolak (bukan https atau domain di luar allowlist).
  const rejected = failure instanceof ApiError && failure.status === 422;

  return (
    <form noValidate onSubmit={(e) => void submit(e)} className="flex max-w-xl flex-col gap-4">
      {failure !== null && !rejected ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}
      <TextField
        label={t('webhook.page.address')}
        description={t('webhook.address.help')}
        value={url}
        onChange={(v) => {
          setUrl(v);
          setFailure(null);
        }}
        inputMode="url"
        autoComplete="off"
        isInvalid={(attempted && urlError !== null) || rejected}
        errorMessage={rejected ? t('clients.webhook.rejected') : urlError}
      />
      <Switch isSelected={aktif} onChange={setAktif}>
        {t('clients.webhook.active')}
      </Switch>
      <div>
        <Button isDisabled={saving} onPress={() => void submit()}>
          {saving ? t('provider.form.saving') : t('common.action.save')}
        </Button>
      </div>
    </form>
  );
}

function RotateDialog({
  namaClient,
  busy,
  failure,
  onConfirm,
  onClose,
}: {
  namaClient: string;
  busy: boolean;
  failure: unknown;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  return (
    <Dialog
      isOpen
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t('webhook.rotate.title')}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onClose}>
            {t('common.action.cancel')}
          </Button>
          <Button destructive isDisabled={busy} onPress={onConfirm}>
            {t('webhook.secret.rotate')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 pt-2">
        <p>{t('webhook.rotate.body', { nama_client: namaClient })}</p>
        {failure !== null ? (
          <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
        ) : null}
      </div>
    </Dialog>
  );
}

function hasilTone(hasil: WebhookLog['hasil']) {
  return hasil === 'terkirim' ? 'success' : 'error';
}

export interface WebhookScreenProps {
  clientId: string;
  client?: ApiClient;
}

/**
 * S09 konfigurasi webhook dan S09a riwayat pengiriman satu client. Webhook yang belum pernah
 * diatur (404) adalah keadaan kosong, bukan galat. Secret tanda tangan hanya tampil sekali, saat
 * dibuat atau dirotasi.
 */
export function WebhookScreen({ clientId, client = api }: WebhookScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const fmt = useFormatter();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.webhook'));
  const allowed = canViewClients(role);
  const [hasil, setHasil] = useState<Hasil>('semua');
  // Jendela 24 jam dihitung dari saat layar dibuka; muat ulang halaman untuk menggesernya.
  const [bukaPada] = useState(() => Date.now());
  const [rotating, setRotating] = useState(false);
  const [rotateBusy, setRotateBusy] = useState(false);
  const [rotateFailure, setRotateFailure] = useState<unknown>(null);
  const [secret, setSecret] = useState<WebhookSecret | null>(null);
  const [secretError, setSecretError] = useState<unknown>(null);

  const query = useQuery<{ client: Client; config: WebhookConfig | null }>(
    async (signal) => {
      const info = await client.getClient(clientId, { signal });
      try {
        return { client: info, config: await client.getWebhookConfig(clientId, { signal }) };
      } catch (error) {
        if (error instanceof ApiError && error.status === 404)
          return { client: info, config: null };
        throw error;
      }
    },
    `webhook:${clientId}`,
    { enabled: allowed },
  );
  const log = useQuery<WebhookLog[]>(
    (signal) => client.listWebhookLog(clientId, hasil === 'semua' ? {} : { hasil }, { signal }),
    `webhook-log:${clientId}:${hasil}`,
    { enabled: allowed },
  );

  if (!allowed) return <Forbidden />;

  const data = query.status === 'success' ? query.data : null;
  const nama = data?.client.nama ?? '';
  const rows = log.status === 'success' ? log.data : [];
  const sehari = bukaPada - HARI_MS;
  const gagal24 = rows.filter(
    (r) => r.hasil !== 'terkirim' && new Date(r.sent_at).getTime() >= sehari,
  ).length;

  async function rotate(fromDialog: boolean) {
    if (rotateBusy) return;
    setRotateBusy(true);
    setRotateFailure(null);
    setSecretError(null);
    try {
      setSecret(await client.rotateWebhookSecret(clientId));
      setRotating(false);
      query.reload();
    } catch (error) {
      if (fromDialog) setRotateFailure(error);
      else setSecretError(error);
    }
    setRotateBusy(false);
  }

  const ada = data?.config?.secret_versi != null;

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={t('webhook.page.title', { nama_client: nama })}
        subtitle={t('clients.webhook.subtitle')}
        titleRef={titleRef}
      >
        <ClientSubnav clientId={clientId} active="webhook" />
      </PageHeader>

      {query.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {query.status === 'error' ? <QueryError error={query.error} onRetry={query.reload} /> : null}

      {data ? (
        <Card variant="outlined" className="flex flex-col gap-4 p-6">
          {data.config === null ? (
            <InlineAlert tone="info">{t('clients.webhook.not_configured')}</InlineAlert>
          ) : null}
          <ConfigForm
            key={`${data.config?.webhook_url ?? ''}|${data.config?.aktif ?? ''}`}
            client={client}
            clientId={clientId}
            config={data.config}
            onSaved={() => {
              snackbar.notify(t('webhook.snackbar.saved'));
              query.reload();
            }}
          />
          <div className="flex flex-col gap-2 border-t border-outline-variant pt-4">
            <h2 className="typescale-title-medium">{t('webhook.secret.heading')}</h2>
            <p className="typescale-body-medium text-on-surface-variant">
              {t('webhook.secret.desc')}
            </p>
            <p className="typescale-body-medium">
              {ada
                ? t('clients.webhook.secret_version', { versi: data.config?.secret_versi })
                : t('clients.webhook.no_secret')}
            </p>
            {secretError !== null ? (
              <InlineAlert tone="error">{errorMessage(t, i18n, secretError).message}</InlineAlert>
            ) : null}
            <div>
              {ada ? (
                <Button variant="outlined" onPress={() => setRotating(true)}>
                  {t('webhook.secret.rotate')}
                </Button>
              ) : (
                <Button
                  variant="outlined"
                  isDisabled={rotateBusy}
                  onPress={() => void rotate(false)}
                >
                  {t('webhook.secret.create')}
                </Button>
              )}
            </div>
          </div>
        </Card>
      ) : null}

      <Card variant="outlined" className="flex flex-col gap-4 p-6">
        <h2 className="typescale-title-large">{t('clients.webhook.log.heading')}</h2>
        <Select
          label={t('clients.webhook.log.filter')}
          selectedKey={hasil}
          onSelectionChange={(key) =>
            setHasil(key === 'terkirim' || key === 'gagal' ? key : 'semua')
          }
        >
          <SelectItem id="semua">{t('common.action.all')}</SelectItem>
          <SelectItem id="terkirim">{t('clients.webhook.hasil.terkirim')}</SelectItem>
          <SelectItem id="gagal">{t('clients.webhook.log.failed_any')}</SelectItem>
        </Select>
        {gagal24 > 0 ? (
          <InlineAlert tone="warning" heading={t('webhook.failures.heading')}>
            {t('webhook.failures.alert', { count: gagal24 })}
          </InlineAlert>
        ) : null}
        {log.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
        {log.status === 'error' ? <QueryError error={log.error} onRetry={log.reload} /> : null}
        {log.status === 'success' && rows.length === 0 ? (
          <p className="typescale-body-medium text-on-surface-variant">
            {t('clients.webhook.log.empty')}
          </p>
        ) : null}
        {rows.length > 0 ? (
          <ScrollArea label={t('clients.webhook.log.heading')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('clients.webhook.log.heading')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('webhook.col.time')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('clients.webhook.log.col.request')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('clients.webhook.log.col.result')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('webhook.col.reason')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('clients.webhook.log.col.version')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('webhook.col.note')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr
                      key={`${row.request_id}|${row.sent_at}`}
                      className="border-b border-outline-variant"
                    >
                      <td data-label={t('webhook.col.time')} className={TD}>
                        {fmt.tanggalWaktu(row.sent_at)}
                      </td>
                      <td data-label={t('clients.webhook.log.col.request')} className={TD}>
                        <span className="break-all font-code">{row.request_id}</span>
                      </td>
                      <td data-label={t('clients.webhook.log.col.result')} className={TD}>
                        <StatusLabel tone={hasilTone(row.hasil)}>
                          {t(`clients.webhook.hasil.${row.hasil}`)}
                        </StatusLabel>
                      </td>
                      <td data-label={t('webhook.col.reason')} className={TD}>
                        {row.hasil === 'terkirim' ? (
                          '—'
                        ) : (
                          <>
                            {row.http_status != null
                              ? t('webhook.reason.http', { kode: row.http_status })
                              : null}
                            {row.error_class ? (
                              <span className="block">
                                {t(`clients.webhook.kelas.${row.error_class}`)}
                              </span>
                            ) : null}
                          </>
                        )}
                      </td>
                      <td data-label={t('clients.webhook.log.col.version')} className={TD}>
                        {row.secret_versi ?? '—'}
                      </td>
                      <td data-label={t('webhook.col.note')} className={TD}>
                        {row.hasil === 'terkirim' ? '' : t('webhook.note.not_resent')}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        ) : null}
        {rows.length > 0 ? (
          <p className="typescale-body-small text-on-surface-variant">
            {t('clients.webhook.log.first_page')}
          </p>
        ) : null}
      </Card>

      {rotating ? (
        <RotateDialog
          namaClient={nama}
          busy={rotateBusy}
          failure={rotateFailure}
          onConfirm={() => void rotate(true)}
          onClose={() => setRotating(false)}
        />
      ) : null}
      {secret ? (
        <OnceSecretDialog
          title={t('webhook.created.title', { nama_client: nama })}
          fieldLabel={t('webhook.created.field')}
          value={secret.secret}
          onDone={() => setSecret(null)}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
