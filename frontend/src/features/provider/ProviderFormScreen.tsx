import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { i18n as I18n } from 'i18next';
import { api, ApiError, type ApiClient, type Provider } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { TextArea, TextField } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from './Forbidden';
import { LinkButton } from './LinkButton';
import {
  ALASAN_MAKS,
  CONFIG_SCHEMA_VERSIONS,
  draftFromProvider,
  emptyDraft,
  firstInvalid,
  LOG_MODE,
  METODE,
  serverErrors,
  TLS_TRUST,
  toProviderConfig,
  type DraftErrors,
  type DraftField,
  type FieldIssue,
  type ProviderDraft,
} from './providerForm';

/** `t` dengan kunci dinamis; kunci sudah dicek dengan `exists` sebelum dipanggil. */
function tDynamic(instance: I18n, key: string, vars: Record<string, unknown> = {}): string {
  return (instance.t as (k: string, o?: Record<string, unknown>) => string)(key, vars);
}

/** Kalimat galat per field dari katalog; kode yang belum dikenal memakai kalimat umum. */
export function issueMessage(instance: I18n, issue: FieldIssue): string {
  const vars = {
    kode: issue.kode,
    lokasi: issue.lokasi ?? '',
    nama: issue.nama ?? '',
    baris: issue.posisi?.baris ?? '',
    kolom: issue.posisi?.kolom ?? '',
  };
  if (issue.kode === 'template_secret_tidak_ada' && issue.nama) {
    return tDynamic(instance, 'provider.template.error.secret_missing', vars);
  }
  if (issue.kode === 'template_tidak_seimbang' && issue.posisi) {
    return tDynamic(instance, 'provider.form.error.template_tidak_seimbang_posisi', vars);
  }
  const key = `provider.form.error.${issue.kode}`;
  if (instance.exists(key)) return tDynamic(instance, key, vars);
  return tDynamic(instance, 'provider.form.error.umum', vars);
}

/** Fokus ke kontrol pertama di pembungkus field (input, textarea, atau tombol select). */
function focusField(field: DraftField): void {
  const box = document.querySelector(`[data-field="${field}"]`);
  box?.querySelector<HTMLElement>('input, textarea, button')?.focus();
}

interface ProviderFormProps {
  client: ApiClient;
  /** Provider yang sedang disunting; `null` = membuat baru. */
  initial: Provider | null;
}

function ProviderForm({ client, initial }: ProviderFormProps): ReactNode {
  const { t, i18n } = useTranslation();
  const snackbar = useSnackbar();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const [current, setCurrent] = useState<Provider | null>(initial);
  const [draft, setDraft] = useState<ProviderDraft>(() =>
    initial ? draftFromProvider(initial) : emptyDraft(),
  );
  const [errors, setErrors] = useState<DraftErrors>({});
  const [lainnya, setLainnya] = useState<FieldIssue[]>([]);
  const [failure, setFailure] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);

  const mode = current ? 'update' : 'create';
  useDocumentTitle(t('title.page.provider'));

  function set<K extends DraftField>(field: K, value: ProviderDraft[K]) {
    setDraft((d) => ({ ...d, [field]: value }));
    // Galat field hilang begitu admin mengubah isinya; galat lain tetap sampai simpan berikutnya.
    setErrors((e) => {
      if (e[field] === undefined) return e;
      const next = { ...e };
      delete next[field];
      return next;
    });
  }

  function showErrors(next: DraftErrors) {
    setErrors(next);
    const first = firstInvalid(next);
    if (first) queueMicrotask(() => focusField(first));
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (saving) return;
    setFailure(null);
    setLainnya([]);
    const built = toProviderConfig(draft, mode);
    if (!built.ok) {
      showErrors(built.errors);
      return;
    }
    setErrors({});
    setSaving(true);
    try {
      const saved = current
        ? await client.updateProvider(current.provider_id, built.body)
        : await client.createProvider(built.body);
      setCurrent(saved);
      // Form diisi ulang dari jawaban server; `alasan` dikosongkan (tidak disimpan, AC-10).
      setDraft(draftFromProvider(saved));
      if (!current && typeof window !== 'undefined') {
        window.history.replaceState(null, '', `/provider/${encodeURIComponent(saved.provider_id)}`);
      }
      snackbar.notify(
        t('provider.form.saved_version', {
          nama: saved.nama,
          versi: saved.provider_config_version,
        }),
      );
    } catch (error) {
      const mapped =
        error instanceof ApiError && error.status === 422 ? serverErrors(error.problem) : null;
      if (mapped && Object.keys(mapped.fields).length > 0) {
        setLainnya(mapped.lainnya);
        showErrors(mapped.fields);
      } else if (mapped) {
        setLainnya(mapped.lainnya);
      } else {
        setFailure(error);
      }
    } finally {
      setSaving(false);
    }
  }

  const invalidCount = Object.keys(errors).length;
  const err = (field: DraftField) => {
    const issue = errors[field];
    return {
      isInvalid: issue !== undefined,
      errorMessage: issue ? issueMessage(i18n, issue) : undefined,
    };
  };
  const showDataBalikan = draft.config_schema_version === 2 || draft.data_balikan.trim() !== '';

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={
          current ? t('provider.form.title_edit', { nama: current.nama }) : t('provider.page.add')
        }
        subtitle={t('provider.form.subtitle')}
        titleRef={titleRef}
        actions={
          <LinkButton href="/provider" variant="text">
            {t('provider.form.back')}
          </LinkButton>
        }
      />

      {invalidCount > 0 ? (
        <InlineAlert tone="error">
          {t('provider.form.error.summary', { jumlah: invalidCount })}
        </InlineAlert>
      ) : null}
      {lainnya.length > 0 ? (
        <InlineAlert tone="error">
          <ul className="list-disc pl-5">
            {lainnya.map((issue) => (
              <li key={`${issue.lokasi}-${issue.kode}`}>{issueMessage(i18n, issue)}</li>
            ))}
          </ul>
        </InlineAlert>
      ) : null}
      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}

      <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-4">
        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">{t('provider.tab.general')}</h2>
          <div data-field="nama">
            <TextField
              label={t('provider.general.name')}
              value={draft.nama}
              onChange={(v) => set('nama', v)}
              autoComplete="off"
              {...err('nama')}
            />
          </div>
          <div data-field="base_url">
            <TextField
              label={t('provider.general.url')}
              description={t('provider.form.url_help')}
              type="url"
              value={draft.base_url}
              onChange={(v) => set('base_url', v)}
              autoComplete="off"
              {...err('base_url')}
            />
          </div>
          <div className="flex flex-wrap gap-4">
            <div data-field="metode">
              <Select
                label={t('provider.general.method')}
                selectedKey={draft.metode}
                onSelectionChange={(key) => {
                  const found = METODE.find((m) => m === key);
                  if (found) set('metode', found);
                }}
              >
                {METODE.map((m) => (
                  <SelectItem key={m} id={m} textValue={m}>
                    {m}
                  </SelectItem>
                ))}
              </Select>
            </div>
            <div data-field="tls_trust">
              <Select
                label={t('provider.form.tls.label')}
                selectedKey={draft.tls_trust}
                onSelectionChange={(key) => {
                  const found = TLS_TRUST.find((v) => v === key);
                  if (found) set('tls_trust', found);
                }}
              >
                {TLS_TRUST.map((v) => (
                  <SelectItem key={v} id={v}>
                    {tDynamic(i18n, `provider.form.tls.${v}`)}
                  </SelectItem>
                ))}
              </Select>
            </div>
            <div data-field="log_mode">
              <Select
                label={t('provider.form.log_mode.label')}
                selectedKey={draft.log_mode}
                onSelectionChange={(key) => {
                  const found = LOG_MODE.find((v) => v === key);
                  if (found) set('log_mode', found);
                }}
              >
                {LOG_MODE.map((v) => (
                  <SelectItem key={v} id={v}>
                    {tDynamic(i18n, `provider.form.log_mode.${v}`)}
                  </SelectItem>
                ))}
              </Select>
            </div>
            <div data-field="config_schema_version">
              <Select
                label={t('provider.form.version.label')}
                selectedKey={draft.config_schema_version}
                onSelectionChange={(key) => {
                  const found = CONFIG_SCHEMA_VERSIONS.find((v) => v === key);
                  if (found !== undefined) set('config_schema_version', found);
                }}
              >
                {CONFIG_SCHEMA_VERSIONS.map((v) => (
                  <SelectItem key={v} id={v}>
                    {tDynamic(i18n, `provider.form.version.v${v}`)}
                  </SelectItem>
                ))}
              </Select>
            </div>
          </div>
        </Card>

        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">{t('provider.tab.template')}</h2>
          <div data-field="body_template">
            <TextArea
              label={t('provider.template.label')}
              description={t('provider.template.help')}
              value={draft.body_template}
              onChange={(v) => set('body_template', v)}
              rows={8}
              textAreaClassName="font-code"
              {...err('body_template')}
            />
          </div>
        </Card>

        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">{t('provider.tab.rules')}</h2>
          <div data-field="parameters">
            <TextArea
              label={t('provider.form.parameters.label')}
              description={t('provider.form.parameters.help')}
              value={draft.parameters}
              onChange={(v) => set('parameters', v)}
              rows={8}
              textAreaClassName="font-code"
              {...err('parameters')}
            />
          </div>
        </Card>

        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">{t('provider.tab.schema')}</h2>
          <div data-field="respons_skema">
            <TextArea
              label={t('provider.schema.label')}
              description={t('provider.schema.help')}
              value={draft.respons_skema}
              onChange={(v) => set('respons_skema', v)}
              rows={8}
              textAreaClassName="font-code"
              {...err('respons_skema')}
            />
          </div>
        </Card>

        {showDataBalikan ? (
          <Card variant="outlined" className="flex flex-col gap-4">
            <h2 className="typescale-title-medium">{t('provider.form.data_balikan.heading')}</h2>
            <div data-field="data_balikan">
              <TextArea
                label={t('provider.form.data_balikan.label')}
                description={t('provider.form.data_balikan.help')}
                value={draft.data_balikan}
                onChange={(v) => set('data_balikan', v)}
                rows={8}
                textAreaClassName="font-code"
                {...err('data_balikan')}
              />
            </div>
          </Card>
        ) : null}

        {mode === 'update' ? (
          <Card variant="outlined" className="flex flex-col gap-4">
            <h2 className="typescale-title-medium">{t('provider.form.reason.heading')}</h2>
            <div data-field="alasan">
              <TextArea
                label={t('provider.form.reason.label')}
                description={t('provider.form.reason.help', {
                  jumlah: draft.alasan.trim().length,
                  batas: ALASAN_MAKS,
                })}
                value={draft.alasan}
                onChange={(v) => set('alasan', v)}
                autoComplete="off"
                rows={3}
                {...err('alasan')}
              />
            </div>
          </Card>
        ) : null}

        <div className="flex flex-wrap justify-end gap-2">
          <LinkButton href="/provider" variant="text">
            {t('common.action.cancel')}
          </LinkButton>
          <Button type="submit" isDisabled={saving}>
            {saving
              ? t('provider.form.saving')
              : mode === 'update'
                ? t('provider.form.save_edit')
                : t('provider.form.save_new')}
          </Button>
        </div>
      </form>
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}

export interface ProviderFormScreenProps {
  client?: ApiClient;
  /** `undefined` = provider baru (`/provider/baru`). */
  providerId?: string;
}

/** S03–S04 form provider: buat baru atau sunting versi terbaru (ARC-101). */
export function ProviderFormScreen({ client = api, providerId }: ProviderFormScreenProps) {
  const { t } = useTranslation();
  const role = useRole();
  const allowed = role === null || role === 'platform_admin';
  const editing = providerId !== undefined;
  const provider = useQuery<Provider>(
    (signal) => client.getProvider(providerId ?? '', { signal }),
    `provider:${providerId ?? ''}`,
    { enabled: allowed && editing },
  );

  if (!allowed) return <Forbidden />;
  if (!editing) return <ProviderForm client={client} initial={null} />;
  if (provider.status === 'loading') return <LoadingState label={t('common.state.loading')} />;
  if (provider.status === 'error') {
    return (
      <section className="flex flex-col gap-4">
        <PageHeader title={t('provider.page.title')} />
        <QueryError error={provider.error} onRetry={provider.reload} />
      </section>
    );
  }
  // Form dipasang setelah data terbaca supaya isian awal tidak di-set dari effect.
  return (
    <ProviderForm
      key={`${provider.data.provider_id}:${provider.data.provider_config_version}`}
      client={client}
      initial={provider.data}
    />
  );
}
