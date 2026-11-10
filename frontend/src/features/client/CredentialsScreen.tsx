import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  api,
  ApiError,
  type ApiClient,
  type Client,
  type Credential,
  type CredentialSecret,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { StatusLabel, type StatusLabelTone } from '../../components/m3/StatusLabel';
import { useFormatter } from '../../i18n/useFormatter';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { useCrumbName } from '../common/CrumbName';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { canViewClients, MAKS_KREDENSIAL_AKTIF } from './access';
import { ClientSubnav } from './ClientSubnav';
import { K20Dialog } from './K20Dialog';
import { OnceSecretDialog } from './OnceSecretDialog';

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

/** Pilihan umur kredensial; 365 hari adalah bawaan dan batas atas di server. */
const UMUR_HARI = [365, 180, 90, 30] as const;
const HARI_MS = 24 * 60 * 60 * 1000;

const STATUS_TONE: Record<Credential['status'], StatusLabelTone> = {
  aktif: 'success',
  kedaluwarsa: 'neutral',
  dicabut: 'error',
};

function BuatDialog({
  client,
  clientId,
  onClose,
  onCreated,
}: {
  client: ApiClient;
  clientId: string;
  onClose: () => void;
  onCreated: (secret: CredentialSecret) => void;
}) {
  const { t, i18n } = useTranslation();
  const [jenis, setJenis] = useState<Credential['jenis']>('oauth2');
  const [hari, setHari] = useState<number>(365);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);

  async function buat() {
    if (busy) return;
    setBusy(true);
    setFailure(null);
    try {
      // Umur penuh = bawaan server, jadi tidak dikirim (menghindari selisih jam yang melewati batas).
      onCreated(
        await client.createCredential(clientId, {
          jenis,
          ...(hari === 365
            ? {}
            : { kedaluwarsa_at: new Date(Date.now() + hari * HARI_MS).toISOString() }),
        }),
      );
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
      title={t('credentials.create.title')}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onClose}>
            {t('common.action.cancel')}
          </Button>
          <Button isDisabled={busy} onPress={() => void buat()}>
            {t('client.credential.create')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4 pt-2">
        {failure !== null ? (
          <InlineAlert tone="error">
            {/* Satu-satunya 409 di sini adalah batas dua kredensial aktif. */}
            {failure instanceof ApiError && failure.status === 409
              ? t('credentials.create.max')
              : errorMessage(t, i18n, failure).message}
          </InlineAlert>
        ) : null}
        <Select
          label={t('client.col.type')}
          selectedKey={jenis}
          onSelectionChange={(key) => setJenis(key === 'api_key' ? 'api_key' : 'oauth2')}
        >
          <SelectItem id="oauth2">{t('client.type.oauth')}</SelectItem>
          <SelectItem id="api_key">{t('client.type.api_key')}</SelectItem>
        </Select>
        <Select
          label={t('credentials.create.expiry')}
          selectedKey={String(hari)}
          onSelectionChange={(key) => setHari(Number(key))}
        >
          {UMUR_HARI.map((n) => (
            <SelectItem
              key={n}
              id={String(n)}
              textValue={t('credentials.create.days', { count: n })}
            >
              {t('credentials.create.days', { count: n })}
            </SelectItem>
          ))}
        </Select>
        <p className="typescale-body-small text-on-surface-variant">
          {t('credentials.create.once')}
        </p>
      </div>
    </Dialog>
  );
}

export interface CredentialsScreenProps {
  clientId: string;
  client?: ApiClient;
}

/**
 * S08b kredensial satu client: daftar tanpa hash, buat (secret tampil sekali), dan cabut lewat
 * dialog K-20. Platform admin dan client_owner clientnya sendiri boleh keduanya; server menolak
 * client lain dengan 403.
 */
export function CredentialsScreen({ clientId, client = api }: CredentialsScreenProps): ReactNode {
  const { t } = useTranslation();
  const fmt = useFormatter();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.client'));
  const allowed = canViewClients(role);
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<Credential | null>(null);
  const [secret, setSecret] = useState<CredentialSecret | null>(null);

  const query = useQuery<{ client: Client; items: Credential[] }>(
    async (signal) => {
      const [info, items] = await Promise.all([
        client.getClient(clientId, { signal }),
        client.listCredentials(clientId, { signal }),
      ]);
      return { client: info, items };
    },
    `kredensial:${clientId}`,
    { enabled: allowed },
  );

  useCrumbName(clientId, query.status === 'success' ? query.data.client.nama : undefined);

  if (!allowed) return <Forbidden />;

  const data = query.status === 'success' ? query.data : null;
  const nama = data?.client.nama ?? '';
  const aktifCount = data?.items.filter((c) => c.status === 'aktif').length ?? 0;
  const penuh = aktifCount >= MAKS_KREDENSIAL_AKTIF;
  const buat = (
    <Button isDisabled={penuh} onPress={() => setCreating(true)}>
      {t('client.credential.create')}
    </Button>
  );

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={t('credentials.page.title', { nama_client: nama })}
        subtitle={t('credentials.page.subtitle')}
        titleRef={titleRef}
        actions={
          <>
            <ClientSubnav clientId={clientId} active="kredensial" />
            {data ? buat : null}
          </>
        }
      />

      {query.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {query.status === 'error' ? <QueryError error={query.error} onRetry={query.reload} /> : null}
      {penuh ? <InlineAlert tone="info">{t('credentials.create.max')}</InlineAlert> : null}

      {data && data.items.length === 0 ? (
        <Card variant="outlined" className="flex flex-col items-center gap-3 p-8 text-center">
          <h2 className="typescale-title-large">{t('credentials.empty.title')}</h2>
          <p className="typescale-body-medium max-w-xl text-on-surface-variant">
            {t('credentials.empty.body')}
          </p>
        </Card>
      ) : null}

      {data && data.items.length > 0 ? (
        <Card variant="outlined" className="p-0">
          <ScrollArea label={t('credentials.table')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('credentials.table')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('credentials.col.id')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.type')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.status')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('credentials.col.expires')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {data.items.map((row) => (
                    <tr key={row.credential_id} className="border-b border-outline-variant">
                      <td data-label={t('credentials.col.id')} className={TD}>
                        <span className="break-all font-code">{row.credential_id}</span>
                      </td>
                      <td data-label={t('client.col.type')} className={TD}>
                        {row.jenis === 'api_key'
                          ? t('client.type.api_key')
                          : t('client.type.oauth')}
                      </td>
                      <td data-label={t('client.col.status')} className={TD}>
                        <StatusLabel tone={STATUS_TONE[row.status]}>
                          {t(`credentials.status.${row.status}`)}
                        </StatusLabel>
                      </td>
                      <td data-label={t('credentials.col.expires')} className={TD}>
                        {row.dicabut_at
                          ? t('credentials.revoked_at', { waktu: fmt.tanggalWaktu(row.dicabut_at) })
                          : row.kedaluwarsa_at
                            ? fmt.tanggalWaktu(row.kedaluwarsa_at)
                            : t('credentials.no_expiry')}
                      </td>
                      <td data-label={t('client.col.actions')} className={TD}>
                        {row.status === 'aktif' ? (
                          <Button
                            variant="text"
                            destructive
                            aria-label={t('credentials.revoke_aria', { id: row.credential_id })}
                            onPress={() => setRevoking(row)}
                          >
                            {t('client.credential.revoke')}
                          </Button>
                        ) : null}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        </Card>
      ) : null}

      {creating ? (
        <BuatDialog
          client={client}
          clientId={clientId}
          onClose={() => setCreating(false)}
          onCreated={(created) => {
            setCreating(false);
            setSecret(created);
          }}
        />
      ) : null}
      {secret ? (
        <OnceSecretDialog
          title={t('client.credential.dialog_title', { nama_client: nama })}
          fieldLabel={t('secret_once.dialog.client_secret')}
          value={secret.secret}
          clientId={secret.client_id}
          onDone={() => {
            setSecret(null);
            // Muat ulang setelah dialog tutup: selama terbuka, nama client di judul harus tetap ada.
            query.reload();
          }}
        />
      ) : null}
      {revoking ? (
        <K20Dialog
          title={t('risk.credential.title')}
          confirmLabel={t('risk.credential.confirm')}
          scope={
            <div className="flex flex-col gap-1">
              <p>{t('risk.credential.scope')}</p>
              <p>{t('risk.credential.new_requests')}</p>
              <p>{t('risk.credential.in_flight_result')}</p>
            </div>
          }
          notes={
            <div className="typescale-body-small flex flex-col gap-1 text-on-surface-variant">
              <p>{t('risk.credential.irreversible')}</p>
              <p>{t('risk.common.timing')}</p>
              <p>{t('risk.common.audit')}</p>
            </div>
          }
          loadPreview={(signal) =>
            client.pratinjauCabutCredential(revoking.credential_id, { signal })
          }
          run={(alasan) => client.cabutCredential(revoking.credential_id, alasan)}
          onClose={() => setRevoking(null)}
          onDone={() => {
            setRevoking(null);
            snackbar.notify(t('risk.credential.done'));
            query.reload();
          }}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
