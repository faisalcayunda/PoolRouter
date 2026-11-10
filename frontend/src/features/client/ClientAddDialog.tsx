import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { type ApiClient } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { TextField } from '../../components/m3/TextField';
import { errorMessage } from '../common/page';
import { NAMA_MAKS } from './clientForm';
import { ALAMAT_HTTPS } from './WebhookScreen';

/**
 * Dialog S08b tambah client. Desain memuat nama, jenis kredensial, downstream pool, dan alamat
 * webhook (opsional). API `addClient` baru menerima nama dan limiter, jadi jenis kredensial dan
 * downstream pool tampil sebagai placeholder tak aktif (celah API); alamat webhook disimpan lewat
 * `putWebhookConfig` setelah client terbuat. Limiter memakai bawaan server dan diubah di detail client.
 */
export function ClientAddDialog({
  client,
  onClose,
  onSaved,
}: {
  client: ApiClient;
  onClose: () => void;
  /** `webhookGagal` benar bila client sudah terbuat tetapi alamat webhook-nya gagal disimpan. */
  onSaved: (webhookGagal: boolean) => void;
}): ReactNode {
  const { t, i18n } = useTranslation();
  const [nama, setNama] = useState('');
  const [webhook, setWebhook] = useState('');
  const [attempted, setAttempted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);

  const trimmed = nama.trim();
  const namaError =
    trimmed === ''
      ? t('clients.error.name_required')
      : trimmed.length > NAMA_MAKS
        ? t('clients.error.name_too_long', { maks: NAMA_MAKS })
        : null;
  const alamat = webhook.trim();
  const webhookError =
    alamat !== '' && !ALAMAT_HTTPS.test(alamat) ? t('webhook.address.error') : null;

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    setAttempted(true);
    if (saving || namaError || webhookError) return;
    setSaving(true);
    setFailure(null);
    let baru;
    try {
      baru = await client.addClient({
        nama: trimmed,
        limiter_penolakan_burst: null,
        limiter_penolakan_isi_ulang_per_detik: null,
      });
    } catch (error) {
      setFailure(error);
      setSaving(false);
      return;
    }
    // Client sudah ada; kegagalan webhook tidak boleh membuat pengguna menyimpan ulang client yang sama.
    let webhookGagal = false;
    if (alamat !== '') {
      try {
        await client.putWebhookConfig(baru.client_id, { webhook_url: alamat, aktif: true });
      } catch {
        webhookGagal = true;
      }
    }
    onSaved(webhookGagal);
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
        <TextField
          label={t('client.form.name')}
          description={t('client.form.name_help')}
          value={nama}
          onChange={setNama}
          autoComplete="off"
          isInvalid={attempted && namaError !== null}
          errorMessage={namaError}
        />
        <Select
          label={t('client.form.cred_type')}
          selectedKey="oauth"
          isDisabled
          className="[&>button]:w-full"
        >
          <SelectItem id="oauth">{t('client.type.oauth')}</SelectItem>
          <SelectItem id="api_key">{t('client.type.api_key')}</SelectItem>
        </Select>
        <Select label={t('client.form.pool')} isDisabled className="[&>button]:w-full">
          <SelectItem id="none">{t('client.form.pool')}</SelectItem>
        </Select>
        <p className="typescale-body-small -mt-3 text-on-surface-variant">
          {t('client.form.pool_help')}
        </p>
        <TextField
          label={t('client.form.webhook')}
          description={t('client.form.webhook_help')}
          value={webhook}
          onChange={setWebhook}
          inputMode="url"
          autoComplete="off"
          isInvalid={attempted && webhookError !== null}
          errorMessage={webhookError}
        />
      </form>
    </Dialog>
  );
}
