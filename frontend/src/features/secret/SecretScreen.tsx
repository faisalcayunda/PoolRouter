import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { api, ApiError, type ApiClient, type SecretRingkas } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { Dialog } from '../../components/m3/Dialog';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { TextField } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { ScrollArea } from '../common/ScrollArea';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';

/** Aturan nama secret di server (`periksa_nama`): huruf di depan, lalu huruf/angka/_, maks 64. */
export const NAMA_SECRET = /^[A-Za-z][A-Za-z0-9_]{0,63}$/;

const TH = 'border-b border-outline-variant px-4 py-3 font-medium';
const TD = 'typescale-body-medium px-4 py-3 align-middle';

type Dialogs =
  | { jenis: 'tambah' }
  | { jenis: 'ganti'; nama: string }
  | { jenis: 'hapus'; nama: string }
  | { jenis: 'dipakai'; nama: string; jumlah: number }
  | null;

/**
 * Dialog tulis nilai secret (S05 tambah, S05b ganti). Nilai hanya hidup di state dialog ini dan
 * hilang saat dialog ditutup (komponen di-unmount), jadi tidak pernah tertinggal di DOM, cache,
 * URL, atau Snackbar setelah disimpan (AC-14, R-16).
 */
function NilaiDialog({
  client,
  nama: namaTetap,
  existing,
  onClose,
  onSaved,
}: {
  client: ApiClient;
  /** Terisi = ganti nilai secret ini (S05b); kosong = tambah secret baru (S05). */
  nama?: string;
  existing: readonly string[];
  onClose: () => void;
  onSaved: (nama: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const ganti = namaTetap !== undefined;
  const [nama, setNama] = useState(namaTetap ?? '');
  const [nilai, setNilai] = useState('');
  const [attempted, setAttempted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [serverNamaError, setServerNamaError] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);

  const namaError = ganti
    ? null
    : serverNamaError || !NAMA_SECRET.test(nama)
      ? t('secret.error.name_format')
      : existing.includes(nama)
        ? t('secret.error.name_exists')
        : null;
  const nilaiError = nilai === '' ? t('secret.error.value_required') : null;

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    setAttempted(true);
    if (saving || namaError || nilaiError) return;
    setSaving(true);
    setFailure(null);
    try {
      await client.putSecret(nama, nilai);
      setNilai('');
      onSaved(nama);
    } catch (error) {
      if (
        error instanceof ApiError &&
        error.status === 422 &&
        error.problem?.errors?.some((e) => e.field.replace(/^\//, '') === 'nama')
      ) {
        setServerNamaError(true);
      } else {
        setFailure(error);
      }
      setSaving(false);
    }
  }

  return (
    <Dialog
      isOpen
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={ganti ? t('secret.replace.title', { nama: namaTetap }) : t('secret.page.add')}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onClose}>
            {t('common.action.cancel')}
          </Button>
          <Button isDisabled={saving} onPress={() => void submit()}>
            {saving
              ? t('provider.form.saving')
              : ganti
                ? t('secret.replace.confirm')
                : t('secret.add.save')}
          </Button>
        </>
      }
    >
      <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-4 pt-2">
        {failure !== null ? (
          <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
        ) : null}
        {ganti ? null : (
          <TextField
            label={t('secret.col.name')}
            description={t('secret.add.name_help')}
            value={nama}
            onChange={(v) => {
              setNama(v);
              setServerNamaError(false);
            }}
            autoComplete="off"
            isInvalid={attempted && namaError !== null}
            errorMessage={namaError}
            className="[&_input]:font-code"
          />
        )}
        <TextField
          label={ganti ? t('secret.replace.new_value') : t('secret.add.value')}
          description={t('secret.page.write_only')}
          type="password"
          value={nilai}
          onChange={setNilai}
          autoComplete="new-password"
          isInvalid={attempted && nilaiError !== null}
          errorMessage={nilaiError}
        />
      </form>
    </Dialog>
  );
}

function HapusDialog({
  client,
  nama,
  onClose,
  onDeleted,
}: {
  client: ApiClient;
  nama: string;
  onClose: () => void;
  onDeleted: (nama: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<unknown>(null);
  const dipakai = failure instanceof ApiError && failure.kode === 'secret_dipakai' ? failure : null;

  async function hapus() {
    if (busy) return;
    setBusy(true);
    setFailure(null);
    try {
      await client.deleteSecret(nama);
      onDeleted(nama);
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
      title={t('secret.delete.title', { nama })}
      closeLabel={t('common.close')}
      footer={
        <>
          <Button variant="text" onPress={onClose}>
            {t('common.action.cancel')}
          </Button>
          {dipakai ? null : (
            <Button destructive isDisabled={busy} onPress={() => void hapus()}>
              {t('secret.delete.confirm')}
            </Button>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-3 pt-2">
        <p>{t('secret.delete.body')}</p>
        {dipakai ? (
          <InlineAlert tone="warning" heading={t('secret.delete.blocked.title')}>
            <p>{t('secret.delete.blocked.short', { nama })}</p>
            {dipakai.problem?.detail ? <p>{dipakai.problem.detail}</p> : null}
          </InlineAlert>
        ) : failure !== null ? (
          <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
        ) : null}
      </div>
    </Dialog>
  );
}

/**
 * S05 keadaan "Secret masih dipakai": dibuka langsung dari Hapus bila daftar sudah menunjukkan
 * pemakai aktif, jadi tidak perlu memanggil API. Daftar API hanya memuat jumlah provider, bukan
 * namanya (celah desain), maka slot daftar provider berisi jumlahnya.
 */
function DipakaiDialog({
  nama,
  jumlah,
  onClose,
}: {
  nama: string;
  jumlah: number;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Dialog
      isOpen
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t('secret.delete.blocked.title')}
      closeLabel={t('common.close')}
      footer={<Button onPress={onClose}>{t('secret.delete.blocked.ack')}</Button>}
    >
      <p className="pt-2">
        {t('secret.delete.blocked.body', {
          nama,
          daftar: t('secret.used_by', { count: jumlah }),
        })}
      </p>
    </Dialog>
  );
}

export interface SecretScreenProps {
  client?: ApiClient;
}

/**
 * S05 dan S05b kelola secret provider (ARC-101). Daftar hanya memuat nama dan jumlah provider
 * pemakai; server tidak pernah mengirim nilai, dan layar ini tidak punya aksi Lihat atau Salin.
 */
export function SecretScreen({ client = api }: SecretScreenProps): ReactNode {
  const { t } = useTranslation();
  const role = useRole();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const snackbar = useSnackbar();
  useDocumentTitle(t('title.page.secret'));
  const allowed = role === null || role === 'platform_admin';
  const [dialog, setDialog] = useState<Dialogs>(null);

  const secrets = useQuery<SecretRingkas[]>((signal) => client.listSecrets({ signal }), 'secrets', {
    enabled: allowed,
  });

  if (!allowed) return <Forbidden />;

  const rows = secrets.status === 'success' ? secrets.data : [];
  const names = rows.map((row) => row.nama);
  const tambah = (
    <Button onPress={() => setDialog({ jenis: 'tambah' })}>{t('secret.page.add')}</Button>
  );

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={t('secret.page.title')}
        subtitle={t('secret.page.subtitle')}
        titleRef={titleRef}
        actions={rows.length > 0 ? tambah : null}
      />

      {secrets.status === 'loading' ? <LoadingState label={t('common.state.loading')} /> : null}
      {secrets.status === 'error' ? (
        <QueryError error={secrets.error} onRetry={secrets.reload} />
      ) : null}

      {secrets.status === 'success' && rows.length === 0 ? (
        <Card variant="container" className="flex flex-col items-center gap-3 p-8 text-center">
          <h2 className="typescale-title-large text-on-surface">{t('secret.empty.title')}</h2>
          <p className="typescale-body-medium max-w-xl text-on-surface-variant">
            {t('secret.empty.body')}
          </p>
          {tambah}
        </Card>
      ) : null}

      {rows.length > 0 ? (
        <Card variant="container" className="overflow-hidden">
          <ScrollArea label={t('secret.page.title')}>
            <div className="table-card">
              <table className="w-full border-collapse text-left">
                <caption className="sr-only">{t('secret.page.title')}</caption>
                <thead>
                  <tr className="typescale-title-small text-on-surface-variant">
                    <th scope="col" className={TH}>
                      {t('secret.col.name')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('secret.col.value')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('secret.col.used_by')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('secret.col.changed')}
                    </th>
                    <th scope="col" className={TH}>
                      {t('secret.col.actions')}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={row.nama} className="border-b border-outline-variant text-on-surface">
                      <td data-label={t('secret.col.name')} className={TD}>
                        <span className="break-all">{row.nama}</span>
                      </td>
                      <td data-label={t('secret.col.value')} className={TD}>
                        {/* Selalu tersamar: tidak ada nilai di respons, jadi tidak ada yang bisa ditampilkan. */}
                        <span className="text-on-surface-variant" aria-hidden="true">
                          {t('secret.masked')}
                        </span>
                        <span className="sr-only">{t('secret.masked_label')}</span>
                      </td>
                      <td data-label={t('secret.col.used_by')} className={TD}>
                        {row.dipakai_oleh_provider_count > 0
                          ? t('secret.used_by', { count: row.dipakai_oleh_provider_count })
                          : t('secret.unused')}
                      </td>
                      <td data-label={t('secret.col.changed')} className={TD}>
                        {/* Celah desain: GET /secrets belum mengirim waktu ubah (kolom "Diubah" di S05). */}
                        <span className="text-on-surface-variant">
                          {t('secret.changed_unknown')}
                        </span>
                      </td>
                      <td data-label={t('secret.col.actions')} className={TD}>
                        <div className="flex flex-wrap gap-1">
                          <Button
                            variant="text"
                            aria-label={t('secret.row.replace_aria', { nama: row.nama })}
                            onPress={() => setDialog({ jenis: 'ganti', nama: row.nama })}
                          >
                            {t('secret.page.replace')}
                          </Button>
                          <Button
                            variant="text"
                            destructive
                            aria-label={t('secret.row.delete_aria', { nama: row.nama })}
                            onPress={() =>
                              setDialog(
                                row.dipakai_oleh_provider_count > 0
                                  ? {
                                      jenis: 'dipakai',
                                      nama: row.nama,
                                      jumlah: row.dipakai_oleh_provider_count,
                                    }
                                  : { jenis: 'hapus', nama: row.nama },
                              )
                            }
                          >
                            {t('common.action.delete')}
                          </Button>
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

      {dialog?.jenis === 'tambah' || dialog?.jenis === 'ganti' ? (
        <NilaiDialog
          client={client}
          nama={dialog.jenis === 'ganti' ? dialog.nama : undefined}
          existing={names}
          onClose={() => setDialog(null)}
          onSaved={(nama) => {
            const ganti = dialog.jenis === 'ganti';
            setDialog(null);
            snackbar.notify(
              ganti
                ? t('secret.snackbar.replaced', { nama })
                : t('secret.snackbar.saved_named', { nama }),
            );
            secrets.reload();
          }}
        />
      ) : null}
      {dialog?.jenis === 'dipakai' ? (
        <DipakaiDialog nama={dialog.nama} jumlah={dialog.jumlah} onClose={() => setDialog(null)} />
      ) : null}
      {dialog?.jenis === 'hapus' ? (
        <HapusDialog
          client={client}
          nama={dialog.nama}
          onClose={() => setDialog(null)}
          onDeleted={(nama) => {
            setDialog(null);
            snackbar.notify(t('secret.snackbar.deleted', { nama }));
            secrets.reload();
          }}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}
