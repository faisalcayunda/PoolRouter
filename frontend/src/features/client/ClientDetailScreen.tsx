import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, type ApiClient, type Client } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { TextField } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { useCrumbName } from '../common/CrumbName';
import { FieldGrid } from '../common/layout';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { REASON_MAX, REASON_MIN } from '../common/ReasonDialog';
import { useRole } from '../common/RoleContext';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { canManageClients, canViewClients } from './access';
import { formatRefill, parseNumber, validateClientForm, type ClientFormValues } from './clientForm';
import { ClientSubnav } from './ClientSubnav';
import { K20Dialog } from './K20Dialog';
import { LimiterFields } from './LimiterFields';

function initialValues(c: Client, lng: string): ClientFormValues {
  return {
    nama: c.nama,
    burst: String(c.limiter_penolakan_burst),
    refill: formatRefill(c.limiter_penolakan_isi_ulang_per_detik, lng),
  };
}

function EditForm({
  client,
  data,
  onSaved,
  actions,
}: {
  client: ApiClient;
  data: Client;
  onSaved: () => void;
  /** Tombol aktifkan/nonaktifkan (ARC-370 AC2): berdampingan dengan Simpan, satu baris. */
  actions?: ReactNode;
}) {
  const { t, i18n } = useTranslation();
  const [values, setValues] = useState(() => initialValues(data, i18n.language));
  const [alasan, setAlasan] = useState('');
  const [attempted, setAttempted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);

  const errors = validateClientForm(values);
  const burst = parseNumber(values.burst);
  const refill = parseNumber(values.refill);
  // Mengubah limiter menuntut alasan (server menolak tanpa itu); mengubah nama saja tidak.
  const limiterChanged =
    (burst ?? data.limiter_penolakan_burst) !== data.limiter_penolakan_burst ||
    (refill ?? data.limiter_penolakan_isi_ulang_per_detik) !==
      data.limiter_penolakan_isi_ulang_per_detik;
  const trimmed = alasan.trim();
  const alasanError =
    limiterChanged && (trimmed.length < REASON_MIN || trimmed.length > REASON_MAX)
      ? t('log.payload.reason.error')
      : null;

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    setAttempted(true);
    if (saving || Object.keys(errors).length > 0 || alasanError) return;
    setSaving(true);
    setFailure(null);
    try {
      await client.updateClient(data.client_id, {
        nama: values.nama.trim(),
        limiter_penolakan_burst: burst ?? data.limiter_penolakan_burst,
        limiter_penolakan_isi_ulang_per_detik: refill ?? data.limiter_penolakan_isi_ulang_per_detik,
        ...(limiterChanged ? { alasan: trimmed } : {}),
      });
      onSaved();
    } catch (error) {
      setFailure(error);
    }
    setSaving(false);
  }

  return (
    <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-6">
      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}
      <FieldGrid>
        <LimiterFields
          values={values}
          onChange={setValues}
          errors={errors}
          showErrors={attempted}
        />
        {limiterChanged ? (
          <div className="col-span-full">
            <TextField
              label={t('clients.form.reason')}
              description={t('clients.form.reason_help')}
              value={alasan}
              onChange={setAlasan}
              autoComplete="off"
              isRequired
              isInvalid={attempted && alasanError !== null}
              errorMessage={alasanError}
            />
          </div>
        ) : null}
      </FieldGrid>
      <div className="flex flex-wrap items-center gap-3">
        <Button isDisabled={saving} onPress={() => void submit()}>
          {saving ? t('provider.form.saving') : t('common.action.save')}
        </Button>
        {actions}
      </div>
    </form>
  );
}

export interface ClientDetailScreenProps {
  clientId: string;
  client?: ApiClient;
}

/**
 * S08 detail satu client: ubah nama dan limiter (platform admin), aktifkan, dan nonaktifkan lewat
 * dialog K-20. client_owner membaca saja. Penugasan endpoint belum ada di layar ini karena API
 * tidak mengembalikan penugasan saat ini (lihat catatan tiket).
 */
export function ClientDetailScreen({ clientId, client = api }: ClientDetailScreenProps): ReactNode {
  const { t, i18n } = useTranslation();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.client'));
  const allowed = canViewClients(role);
  const [deactivating, setDeactivating] = useState(false);
  const [activateError, setActivateError] = useState<unknown>(null);

  const query = useQuery<Client>(
    (signal) => client.getClient(clientId, { signal }),
    `client:${clientId}`,
    {
      enabled: allowed,
    },
  );

  useCrumbName(clientId, query.status === 'success' ? query.data.nama : undefined);

  if (!allowed) return <Forbidden />;
  const manage = canManageClients(role);
  const data = query.status === 'success' ? query.data : null;

  async function aktifkan() {
    setActivateError(null);
    try {
      await client.aktifkanClient(clientId);
      snackbar.notify(t('clients.activate.done'));
      query.reload();
    } catch (error) {
      setActivateError(error);
    }
  }

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={data?.nama ?? t('client.page.title')}
        subtitle={t('clients.detail.subtitle')}
        titleRef={titleRef}
      >
        <ClientSubnav clientId={clientId} active="detail" />
      </PageHeader>

      {query.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {query.status === 'error' ? <QueryError error={query.error} onRetry={query.reload} /> : null}

      {data ? (
        <Card variant="container" className="flex flex-col gap-6 p-6">
          <div className="flex flex-wrap items-center gap-3">
            <h2 className="typescale-title-large text-primary-text">
              {t('clients.detail.settings')}
            </h2>
            <StatusLabel tone={data.aktif ? 'success' : 'neutral'}>
              {data.aktif ? t('common.state.active') : t('clients.inactive')}
            </StatusLabel>
          </div>
          <p className="typescale-body-medium text-on-surface-variant">
            {t('client.col.id')}: <span className="break-all font-code">{data.client_id}</span>
          </p>

          {manage ? (
            <>
              {activateError !== null ? (
                <InlineAlert tone="error">
                  {errorMessage(t, i18n, activateError).message}
                </InlineAlert>
              ) : null}
              <EditForm
                key={`${data.nama}|${data.limiter_penolakan_burst}|${data.limiter_penolakan_isi_ulang_per_detik}`}
                client={client}
                data={data}
                onSaved={() => {
                  snackbar.notify(t('clients.saved'));
                  query.reload();
                }}
                actions={
                  data.aktif ? (
                    <Button destructive variant="outlined" onPress={() => setDeactivating(true)}>
                      {t('risk.client.confirm')}
                    </Button>
                  ) : (
                    <Button onPress={() => void aktifkan()}>{t('clients.activate.action')}</Button>
                  )
                }
              />
            </>
          ) : (
            <dl className="grid max-w-xl grid-cols-[auto_1fr] gap-x-6 gap-y-2">
              <dt className="text-on-surface-variant">{t('client.form.name')}</dt>
              <dd>{data.nama}</dd>
              <dt className="text-on-surface-variant">{t('clients.col.limiter')}</dt>
              <dd>
                {t('clients.limiter.value', {
                  burst: data.limiter_penolakan_burst,
                  isi_ulang: formatRefill(
                    data.limiter_penolakan_isi_ulang_per_detik,
                    i18n.language,
                  ),
                })}
              </dd>
            </dl>
          )}
        </Card>
      ) : null}

      {deactivating && data ? (
        <K20Dialog
          title={t('risk.client.title')}
          confirmLabel={t('risk.client.confirm')}
          scope={
            <div className="flex flex-col gap-1">
              <p>{t('risk.client.scope')}</p>
              <p>{t('risk.client.new_requests')}</p>
              <p>{t('risk.client.in_flight_result')}</p>
              <p>{t('risk.client.webhook_stays')}</p>
            </div>
          }
          notes={
            <div className="typescale-body-small flex flex-col gap-1 text-on-surface-variant">
              <p>{t('risk.common.timing')}</p>
              <p>{t('risk.common.reversible')}</p>
              <p>{t('risk.common.webhook')}</p>
              <p>{t('risk.common.audit')}</p>
            </div>
          }
          loadPreview={(signal) => client.pratinjauNonaktifClient(clientId, { signal })}
          run={(alasan) => client.nonaktifkanClient(clientId, alasan)}
          onClose={() => setDeactivating(false)}
          onDone={(result) => {
            setDeactivating(false);
            const hasil = result as { jumlah_cancelled: number; jumlah_failed: number };
            snackbar.notify(
              `${t('risk.client.done')} ${t('clients.k20.result', {
                cancelled: hasil.jumlah_cancelled,
                failed: hasil.jumlah_failed,
              })}`,
            );
            query.reload();
          }}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
