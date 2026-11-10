import { useState, type FormEvent, type ReactNode } from 'react';
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
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { TextArea } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { LinkButton } from '../provider/LinkButton';
import {
  ALASAN_MAKS,
  draftFromEndpoint,
  emptyDraft,
  emptyErrors,
  errorCount,
  firstInvalid,
  hasErrors,
  LOG_MODE,
  serverErrors,
  toEndpointConfig,
  type DraftErrors,
  type EndpointDraft,
  type FieldIssue,
  type RowField,
  type TopField,
} from './endpointForm';
import { issueMessage, MetadataFieldsEditor, tDynamic } from './MetadataFieldsEditor';

/** Fokus ke kontrol pertama di pembungkus field (tombol select, input, atau textarea). */
function focusField(field: TopField): void {
  const box = document.querySelector(`[data-field="${field}"]`);
  box?.querySelector<HTMLElement>('input, textarea, button')?.focus();
}

interface Lookups {
  providers: Provider[];
  pools: DownstreamPool[];
}

interface EndpointFormProps {
  client: ApiClient;
  lookups: Lookups;
  /** Endpoint yang sedang disunting; `null` = membuat baru. */
  initial: Endpoint | null;
}

function EndpointForm({ client, lookups, initial }: EndpointFormProps): ReactNode {
  const { t, i18n } = useTranslation();
  const snackbar = useSnackbar();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const [current, setCurrent] = useState<Endpoint | null>(initial);
  const [draft, setDraft] = useState<EndpointDraft>(() =>
    initial ? draftFromEndpoint(initial) : emptyDraft(),
  );
  const [errors, setErrors] = useState<DraftErrors>(emptyErrors);
  const [lainnya, setLainnya] = useState<FieldIssue[]>([]);
  const [failure, setFailure] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);

  const mode = current ? 'update' : 'create';
  useDocumentTitle(t('title.page.endpoint'));

  const provider = lookups.providers.find((p) => p.provider_id === draft.provider_id);

  function set<K extends keyof EndpointDraft>(field: K, value: EndpointDraft[K]) {
    setDraft((d) => ({ ...d, [field]: value }));
    // Galat field hilang begitu admin mengubah isinya; galat lain tetap sampai simpan berikutnya.
    setErrors((e) => {
      if (e.fields[field] === undefined) return e;
      const fields = { ...e.fields };
      delete fields[field];
      return { ...e, fields };
    });
  }

  function touchRow(key: number, field: RowField) {
    setErrors((e) => {
      const row = e.rows[key];
      if (!row || row[field] === undefined) return e;
      const next = { ...row };
      delete next[field];
      const rows = { ...e.rows };
      if (Object.keys(next).length === 0) delete rows[key];
      else rows[key] = next;
      return { ...e, rows };
    });
  }

  function showErrors(next: DraftErrors) {
    setErrors(next);
    const first = firstInvalid(next);
    if (first) {
      queueMicrotask(() => focusField(first));
      return;
    }
    const rowKey = Object.keys(next.rows)[0];
    if (rowKey !== undefined) {
      queueMicrotask(() =>
        document.querySelector<HTMLElement>(`[data-row="${rowKey}"] input`)?.focus(),
      );
    }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (saving) return;
    setFailure(null);
    setLainnya([]);
    const built = toEndpointConfig(draft, mode, provider?.log_mode);
    if (!built.ok) {
      showErrors(built.errors);
      return;
    }
    setErrors(emptyErrors());
    setSaving(true);
    try {
      const saved = current
        ? await client.updateEndpoint(current.endpoint_id, built.body)
        : await client.createEndpoint(built.body);
      setCurrent(saved);
      // Form diisi ulang dari jawaban server; `alasan` dikosongkan.
      setDraft(draftFromEndpoint(saved));
      if (!current && typeof window !== 'undefined') {
        window.history.replaceState(null, '', `/endpoint/${encodeURIComponent(saved.endpoint_id)}`);
      }
      snackbar.notify(t('endpoint.form.saved_version', { versi: saved.endpoint_config_version }));
    } catch (error) {
      const mapped =
        error instanceof ApiError && error.status === 422
          ? serverErrors(error.problem, draft.metadata_fields)
          : null;
      if (mapped) {
        setLainnya(mapped.lainnya);
        if (hasErrors(mapped.errors)) showErrors(mapped.errors);
        else if (mapped.lainnya.length === 0) setFailure(error);
      } else {
        setFailure(error);
      }
    } finally {
      setSaving(false);
    }
  }

  const err = (field: TopField) => {
    const issue = errors.fields[field];
    return {
      isInvalid: issue !== undefined,
      errorMessage: issue ? issueMessage(i18n, issue) : undefined,
    };
  };
  const metadataIssue = errors.fields.metadata_fields;

  return (
    <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-4">
      <PageHeader
        title={
          current && provider
            ? provider.nama
            : current
              ? t('endpoint.form.title_edit')
              : t('endpoint.page.add')
        }
        subtitle={t('endpoint.form.subtitle')}
        titleRef={titleRef}
        actions={
          <>
            <LinkButton href="/endpoint" variant="outlined">
              {t('common.action.cancel')}
            </LinkButton>
            <Button type="submit" isDisabled={saving}>
              {saving
                ? t('endpoint.form.saving')
                : mode === 'update'
                  ? t('endpoint.form.save_edit')
                  : t('endpoint.form.save_new')}
            </Button>
          </>
        }
      />

      <InlineAlert tone="info" heading={t('endpoint.form.notice.title')}>
        {t('endpoint.form.notice.body')}
      </InlineAlert>

      {errorCount(errors) > 0 ? (
        <InlineAlert tone="error">
          {t('endpoint.form.error.summary', { jumlah: errorCount(errors) })}
        </InlineAlert>
      ) : null}
      {lainnya.length > 0 ? (
        <InlineAlert tone="error">
          <ul className="list-disc pl-5">
            {lainnya.map((issue, i) => (
              <li key={`${issue.kode}-${i}`}>{issueMessage(i18n, issue)}</li>
            ))}
          </ul>
        </InlineAlert>
      ) : null}
      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}

      <div className="flex flex-col gap-4">
        <Card variant="container" className="flex flex-col gap-4 p-6">
          <h2 className="typescale-title-medium">{t('endpoint.form.section.target')}</h2>
          <div className="flex flex-wrap gap-4">
            <div data-field="provider_id">
              <Select
                label={t('endpoint.form.provider')}
                placeholder={t('endpoint.form.choose')}
                selectedKey={draft.provider_id || null}
                onSelectionChange={(key) => set('provider_id', key === null ? '' : String(key))}
                isInvalid={err('provider_id').isInvalid}
              >
                {lookups.providers.map((p) => (
                  <SelectItem key={p.provider_id} id={p.provider_id} textValue={p.nama}>
                    {p.nama}
                  </SelectItem>
                ))}
              </Select>
              {err('provider_id').errorMessage ? (
                <p role="alert" className="typescale-body-small text-error">
                  {err('provider_id').errorMessage}
                </p>
              ) : null}
            </div>
            <div data-field="downstream_pool_id">
              <Select
                label={t('endpoint.form.pool')}
                placeholder={t('endpoint.form.choose')}
                selectedKey={draft.downstream_pool_id || null}
                onSelectionChange={(key) =>
                  set('downstream_pool_id', key === null ? '' : String(key))
                }
                isInvalid={err('downstream_pool_id').isInvalid}
              >
                {lookups.pools.map((p) => (
                  <SelectItem
                    key={p.downstream_pool_id}
                    id={p.downstream_pool_id}
                    textValue={p.nama}
                  >
                    {p.nama}
                  </SelectItem>
                ))}
              </Select>
              {err('downstream_pool_id').errorMessage ? (
                <p role="alert" className="typescale-body-small text-error">
                  {err('downstream_pool_id').errorMessage}
                </p>
              ) : null}
            </div>
            <div data-field="log_mode">
              <Select
                label={t('endpoint.form.log_mode.label')}
                selectedKey={draft.log_mode}
                onSelectionChange={(key) => {
                  const found = LOG_MODE.find((v) => v === key);
                  if (found) set('log_mode', found);
                }}
                isInvalid={err('log_mode').isInvalid}
              >
                {LOG_MODE.map((v) => (
                  <SelectItem key={v} id={v}>
                    {tDynamic(i18n, `endpoint.form.log_mode.${v}`)}
                  </SelectItem>
                ))}
              </Select>
              {err('log_mode').errorMessage ? (
                <p role="alert" className="typescale-body-small text-error">
                  {err('log_mode').errorMessage}
                </p>
              ) : (
                <p className="typescale-body-small text-on-surface-variant">
                  {t('endpoint.form.log_mode.help')}
                </p>
              )}
            </div>
          </div>
        </Card>

        <Card variant="container" className="flex flex-col gap-4 p-6">
          <h2 className="typescale-title-medium text-on-surface">
            {provider
              ? `${t('endpoint.form.constants.title')} ${provider.nama}`
              : t('endpoint.form.section.parameters')}
          </h2>
          <div data-field="parameter_literal">
            <TextArea
              label={t('endpoint.form.parameters.label')}
              description={t('endpoint.form.parameters.help')}
              value={draft.parameter_literal}
              onChange={(v) => set('parameter_literal', v)}
              rows={5}
              autoComplete="off"
              textAreaClassName="font-code"
              {...err('parameter_literal')}
            />
          </div>
        </Card>

        <Card variant="container" className="flex flex-col gap-4 p-6">
          <h2 className="typescale-title-medium">{t('endpoint.form.section.metadata')}</h2>
          <p className="typescale-body-medium text-on-surface-variant">
            {t('endpoint.metadata.info')}
          </p>
          <div data-field="metadata_fields">
            {metadataIssue ? (
              <InlineAlert tone="error">{issueMessage(i18n, metadataIssue)}</InlineAlert>
            ) : null}
            <MetadataFieldsEditor
              rows={draft.metadata_fields}
              errors={errors}
              onChange={(rows) => set('metadata_fields', rows)}
              onTouch={touchRow}
            />
          </div>
        </Card>

        {mode === 'update' ? (
          <Card variant="container" className="flex flex-col gap-4 p-6">
            <h2 className="typescale-title-medium">{t('endpoint.form.reason.heading')}</h2>
            <div data-field="alasan">
              <TextArea
                label={t('endpoint.form.reason.label')}
                description={t('endpoint.form.reason.help', {
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
      </div>
      <SnackbarRegion queue={snackbar.queue} />
    </form>
  );
}

export interface EndpointFormScreenProps {
  client?: ApiClient;
  /** `undefined` = endpoint baru (`/endpoint/baru`). */
  endpointId?: string;
}

/** S04b + S07b: form endpoint dengan kontrak metadata; buat baru atau sunting versi terbaru. */
export function EndpointFormScreen({ client = api, endpointId }: EndpointFormScreenProps) {
  const { t } = useTranslation();
  const role = useRole();
  const allowed = role === null || role === 'platform_admin';
  const editing = endpointId !== undefined;
  const data = useQuery<{ lookups: Lookups; endpoint: Endpoint | null }>(
    async (signal) => {
      const [providers, pools, endpoint] = await Promise.all([
        client.listProviders({ signal }),
        client.listDownstreamPools({ signal }),
        editing ? client.getEndpoint(endpointId, { signal }) : Promise.resolve(null),
      ]);
      return { lookups: { providers, pools }, endpoint };
    },
    `endpoint-form:${endpointId ?? ''}`,
    { enabled: allowed },
  );

  if (!allowed) return <Forbidden />;
  if (data.status === 'loading') return <LoadingState label={t('common.state.loading')} />;
  if (data.status === 'error') {
    return (
      <section className="flex flex-col gap-4">
        <PageHeader title={t('endpoint.page.title')} />
        <QueryError error={data.error} onRetry={data.reload} />
      </section>
    );
  }
  // Form dipasang setelah data terbaca supaya isian awal tidak di-set dari effect.
  return (
    <EndpointForm
      key={`${data.data.endpoint?.endpoint_id ?? 'baru'}:${data.data.endpoint?.endpoint_config_version ?? 0}`}
      client={client}
      lookups={data.data.lookups}
      initial={data.data.endpoint}
    />
  );
}
