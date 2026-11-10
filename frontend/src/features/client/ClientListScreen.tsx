import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type Client } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { LinkButton } from '../provider/LinkButton';
import { Forbidden } from '../provider/Forbidden';
import { canManageClients, canViewClients, clientPath } from './access';
import { parseNumber, validateClientForm, type ClientFormValues } from './clientForm';
import { LimiterFields } from './LimiterFields';

// Ukuran mengikuti frame Penpot S08: kepala 40 px berlatar surface, baris 56 px, garis pemisah
// surface-container-high, padding horizontal 20 px.
const TH = 'typescale-label-small h-10 bg-surface px-5 font-medium';
const TD =
  'typescale-body-medium h-14 border-b border-surface-container-high px-5 py-2 align-middle';
const KOSONG = '\u2014';

function TambahDialog({
  client,
  onClose,
  onSaved,
}: {
  client: ApiClient;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [values, setValues] = useState<ClientFormValues>({ nama: '', burst: '', refill: '' });
  const [attempted, setAttempted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const errors = validateClientForm(values);

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    setAttempted(true);
    if (saving || Object.keys(errors).length > 0) return;
    setSaving(true);
    setFailure(null);
    try {
      await client.addClient({
        nama: values.nama.trim(),
        limiter_penolakan_burst: parseNumber(values.burst) ?? null,
        limiter_penolakan_isi_ulang_per_detik: parseNumber(values.refill) ?? null,
      });
      onSaved();
    } catch (error) {
      setFailure(error);
      setSaving(false);
    }
  }

  return (
    <Dialog
      isOpen
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t('client.form.title')}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onClose}>
            {t('common.action.cancel')}
          </Button>
          <Button isDisabled={saving} onPress={() => void submit()}>
            {saving ? t('provider.form.saving') : t('client.form.save')}
          </Button>
        </>
      }
    >
      <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-4 pt-2">
        <p>{t('client.form.intro')}</p>
        {failure !== null ? (
          <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
        ) : null}
        <LimiterFields
          values={values}
          onChange={setValues}
          showErrors={attempted}
          errors={errors}
        />
      </form>
    </Dialog>
  );
}

export interface ClientListScreenProps {
  client?: ApiClient;
}

/**
 * S08 daftar client (ARC-104). Platform admin melihat semua client dan boleh menambah; client_owner
 * hanya melihat clientnya sendiri (server yang membatasi daftar) dan tidak punya aksi tulis.
 */
export function ClientListScreen({ client = api }: ClientListScreenProps): ReactNode {
  const { t } = useTranslation();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.client'));
  const allowed = canViewClients(role);
  const [adding, setAdding] = useState(false);

  const clients = useQuery<Client[]>((signal) => client.listClients({ signal }), 'clients', {
    enabled: allowed,
  });

  if (!allowed) return <Forbidden />;

  const rows = clients.status === 'success' ? clients.data : [];
  const manage = canManageClients(role);
  const tambah = manage ? (
    <Button onPress={() => setAdding(true)}>{t('client.page.add')}</Button>
  ) : null;

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={t('client.page.title')}
        subtitle={t('clients.page.subtitle')}
        titleRef={titleRef}
        actions={rows.length > 0 ? tambah : null}
      />

      {clients.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {clients.status === 'error' ? (
        <QueryError error={clients.error} onRetry={clients.reload} />
      ) : null}

      {clients.status === 'success' && rows.length === 0 ? (
        <Card variant="outlined" className="flex flex-col items-center gap-3 p-8 text-center">
          <h2 className="typescale-title-large text-on-surface">{t('client.empty.title')}</h2>
          <p className="typescale-body-medium max-w-xl text-on-surface-variant">
            {manage ? t('client.empty.body') : t('clients.empty.owner')}
          </p>
          {tambah}
        </Card>
      ) : null}

      {rows.length > 0 ? (
        <Card
          variant="outlined"
          className="overflow-hidden p-0! rounded-large! bg-surface-container-lowest!"
        >
          <ScrollArea label={t('client.page.title')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('client.page.title')}</caption>
                <thead>
                  <tr className="text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('client.col.client')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.type')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.id')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.created')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('client.col.status')}
                    </th>
                    <th scope="col" className={TH}>
                      <span className="sr-only">{t('client.col.actions')}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={row.client_id} className="text-on-surface">
                      <td data-label={t('client.col.client')} className={`${TD} font-medium`}>
                        {row.nama}
                      </td>
                      {/* Belum ada di API: GET /clients tidak memuat jenis kredensial maupun waktu pembuatan. */}
                      <td
                        data-label={t('client.col.type')}
                        className={`${TD} text-on-surface-variant`}
                      >
                        {KOSONG}
                      </td>
                      <td data-label={t('client.col.id')} className={TD}>
                        <span className="break-all font-code">{row.client_id}</span>
                      </td>
                      <td
                        data-label={t('client.col.created')}
                        className={`${TD} text-on-surface-variant`}
                      >
                        {KOSONG}
                      </td>
                      <td data-label={t('client.col.status')} className={TD}>
                        <StatusLabel tone={row.aktif ? 'success' : 'neutral'}>
                          {row.aktif ? t('common.state.active') : t('clients.inactive')}
                        </StatusLabel>
                      </td>
                      <td className={TD}>
                        <div className="flex flex-nowrap items-center justify-end gap-1 whitespace-nowrap max-[599.98px]:justify-start">
                          <LinkButton
                            href={clientPath(row.client_id, 'kredensial')}
                            variant="text"
                            aria-label={t('clients.row.credential_aria', { nama: row.nama })}
                          >
                            {t('client.credential.create')}
                          </LinkButton>
                          <LinkButton
                            href={clientPath(row.client_id)}
                            variant="text"
                            aria-label={t('clients.row.manage_aria', { nama: row.nama })}
                          >
                            {t('clients.row.manage')}
                          </LinkButton>
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

      {adding ? (
        <TambahDialog
          client={client}
          onClose={() => setAdding(false)}
          onSaved={() => {
            setAdding(false);
            snackbar.notify(t('client.snackbar.saved'));
            clients.reload();
          }}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
