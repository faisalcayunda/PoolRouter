import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { i18n as I18n } from 'i18next';
import { api, ApiError, type ApiClient, type Provider, type UpstreamPool } from '../../api/client';
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
import { Forbidden } from '../provider/Forbidden';
import { LinkButton } from '../provider/LinkButton';
import {
  ALASAN_MAKS,
  draftFromPool,
  emptyDraft,
  firstInvalid,
  JENDELA_KUOTA,
  serverErrors,
  toUpstreamConfig,
  type DraftErrors,
  type DraftField,
  type FieldIssue,
  type NumberField,
  type UpstreamDraft,
} from './upstreamForm';

/** `t` dengan kunci dinamis; kunci disusun dari daftar field yang tetap. */
function tDynamic(instance: I18n, key: string, vars: Record<string, unknown> = {}): string {
  return (instance.t as (k: string, o?: Record<string, unknown>) => string)(key, vars);
}

function issueMessage(instance: I18n, issue: FieldIssue): string {
  return tDynamic(instance, `upstream.form.error.${issue.kode}`, { ...issue });
}

function focusField(field: DraftField): void {
  document
    .querySelector(`[data-field="${field}"]`)
    ?.querySelector<HTMLElement>('input, textarea, button')
    ?.focus();
}

/** Kartu per kelompok; urutan field di dalamnya sama dengan `FIELD_ORDER`. */
const GROUPS: { heading: string; fields: NumberField[] }[] = [
  {
    heading: 'upstream.tab.quota',
    fields: [
      'kuota_harian',
      'rate_limit_per_jam',
      'porsi_sync_persen',
      'ambang_berdampingan',
      'jeda_min',
      'jeda_maks',
    ],
  },
  {
    heading: 'upstream.form.group.reset',
    fields: ['reset_kuota_jam'],
  },
  {
    heading: 'upstream.form.group.retry',
    fields: [
      'batas_percobaan',
      'retry_backoff_base_detik',
      'retry_backoff_cap_detik',
      'retry_min_per_jam',
      'retry_rasio',
      'margin_kuota',
    ],
  },
];

interface UpstreamFormProps {
  client: ApiClient;
  providers: Provider[];
  /** Pool yang sedang disunting; `null` = membuat baru. */
  initial: UpstreamPool | null;
}

function UpstreamForm({ client, providers, initial }: UpstreamFormProps): ReactNode {
  const { t, i18n } = useTranslation();
  const snackbar = useSnackbar();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const [current, setCurrent] = useState<UpstreamPool | null>(initial);
  const [draft, setDraft] = useState<UpstreamDraft>(() =>
    initial ? draftFromPool(initial) : emptyDraft(),
  );
  const [errors, setErrors] = useState<DraftErrors>({});
  const [lainnya, setLainnya] = useState(0);
  const [failure, setFailure] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);

  const mode = current ? 'update' : 'create';
  useDocumentTitle(t('title.page.upstream'));

  function set<K extends DraftField>(field: K, value: UpstreamDraft[K]) {
    setDraft((d) => ({ ...d, [field]: value }));
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
    setLainnya(0);
    const built = toUpstreamConfig(draft, mode);
    if (!built.ok) {
      showErrors(built.errors);
      return;
    }
    setErrors({});
    setSaving(true);
    try {
      const saved = current
        ? await client.updateUpstreamPool(current.upstream_pool_id, built.body)
        : await client.createUpstreamPool(built.body);
      setCurrent(saved);
      setDraft(draftFromPool(saved));
      if (!current && typeof window !== 'undefined') {
        window.history.replaceState(
          null,
          '',
          `/upstream/${encodeURIComponent(saved.upstream_pool_id)}`,
        );
      }
      snackbar.notify(t('upstream.form.saved'));
    } catch (error) {
      const mapped =
        error instanceof ApiError && error.status === 422 ? serverErrors(error.problem) : null;
      if (mapped && (Object.keys(mapped.fields).length > 0 || mapped.lainnya > 0)) {
        setLainnya(mapped.lainnya);
        showErrors(mapped.fields);
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
  const providerName = (id: string) => providers.find((p) => p.provider_id === id)?.nama ?? id;
  const alasanWajib = mode === 'update' || draft.jendela_kuota === 'kalender';

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={
          current
            ? t('upstream.form.title_edit', { nama: providerName(current.provider_id) })
            : t('upstream.page.add')
        }
        subtitle={t('upstream.form.subtitle')}
        titleRef={titleRef}
        actions={
          <LinkButton href="/upstream" variant="text">
            {t('upstream.form.back')}
          </LinkButton>
        }
      />

      {invalidCount > 0 ? (
        <InlineAlert tone="error">
          {t('upstream.form.error.summary', { jumlah: invalidCount })}
        </InlineAlert>
      ) : null}
      {lainnya > 0 ? (
        <InlineAlert tone="error">
          {t('upstream.form.error.lainnya', { jumlah: lainnya })}
        </InlineAlert>
      ) : null}
      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}

      <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-4">
        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">{t('upstream.form.group.general')}</h2>
          <div data-field="provider_id">
            <Select
              label={t('upstream.page.provider')}
              selectedKey={draft.provider_id === '' ? null : draft.provider_id}
              onSelectionChange={(key) => {
                const found = providers.find((p) => p.provider_id === key);
                if (found) set('provider_id', found.provider_id);
              }}
              placeholder={t('upstream.form.provider_placeholder')}
              isInvalid={errors.provider_id !== undefined}
            >
              {providers.map((p) => (
                <SelectItem key={p.provider_id} id={p.provider_id} textValue={p.nama}>
                  {p.nama}
                </SelectItem>
              ))}
            </Select>
            {errors.provider_id ? (
              <p className="typescale-body-small text-error">{err('provider_id').errorMessage}</p>
            ) : (
              <p className="typescale-body-small text-on-surface-variant">
                {t('upstream.provider.help')}
              </p>
            )}
          </div>
          <div data-field="jendela_kuota">
            <Select
              label={t('upstream.form.field.jendela_kuota')}
              selectedKey={draft.jendela_kuota}
              onSelectionChange={(key) => {
                const found = JENDELA_KUOTA.find((v) => v === key);
                if (found) set('jendela_kuota', found);
              }}
            >
              {JENDELA_KUOTA.map((v) => (
                <SelectItem key={v} id={v}>
                  {tDynamic(i18n, `upstream.form.jendela.${v}`)}
                </SelectItem>
              ))}
            </Select>
          </div>
        </Card>

        {GROUPS.map((group) => (
          <Card key={group.heading} variant="outlined" className="flex flex-col gap-4">
            <h2 className="typescale-title-medium">{tDynamic(i18n, group.heading)}</h2>
            <div className="grid gap-4 sm:grid-cols-2">
              {group.fields.map((field) => (
                <div key={field} data-field={field}>
                  <TextField
                    label={tDynamic(i18n, `upstream.form.field.${field}`)}
                    value={draft[field]}
                    onChange={(v) => set(field, v)}
                    inputMode="decimal"
                    autoComplete="off"
                    {...err(field)}
                  />
                </div>
              ))}
              {group.heading === 'upstream.form.group.reset' ? (
                <div data-field="reset_kuota_zona">
                  <TextField
                    label={t('upstream.form.field.reset_kuota_zona')}
                    value={draft.reset_kuota_zona}
                    onChange={(v) => set('reset_kuota_zona', v)}
                    autoComplete="off"
                    {...err('reset_kuota_zona')}
                  />
                </div>
              ) : null}
            </div>
          </Card>
        ))}

        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">{t('upstream.form.reason.heading')}</h2>
          <div data-field="alasan">
            <TextArea
              label={
                alasanWajib ? t('upstream.form.reason.label') : t('upstream.form.reason.optional')
              }
              description={t('upstream.form.reason.help', {
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

        <div className="flex flex-wrap justify-end gap-2">
          <LinkButton href="/upstream" variant="text">
            {t('common.action.cancel')}
          </LinkButton>
          <Button type="submit" isDisabled={saving}>
            {saving
              ? t('provider.form.saving')
              : mode === 'update'
                ? t('upstream.form.save_edit')
                : t('upstream.form.save_new')}
          </Button>
        </div>
      </form>
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}

export interface UpstreamFormScreenProps {
  client?: ApiClient;
  /** `undefined` = upstream pool baru (`/upstream/baru`). */
  poolId?: string;
}

/** S06b form upstream pool: buat baru atau sunting konfigurasi saat ini (ARC-102). */
export function UpstreamFormScreen({ client = api, poolId }: UpstreamFormScreenProps) {
  const { t } = useTranslation();
  const role = useRole();
  const allowed = role === null || role === 'platform_admin';
  const editing = poolId !== undefined;
  const providers = useQuery<Provider[]>(
    (signal) => client.listProviders({ signal }),
    'providers',
    {
      enabled: allowed,
    },
  );
  const pool = useQuery<UpstreamPool>(
    (signal) => client.getUpstreamPool(poolId ?? '', { signal }),
    `upstream:${poolId ?? ''}`,
    { enabled: allowed && editing },
  );

  if (!allowed) return <Forbidden />;
  const failed = providers.status === 'error' ? providers : pool.status === 'error' ? pool : null;
  if (failed) {
    return (
      <section className="flex flex-col gap-4">
        <PageHeader title={t('upstream.page.title')} />
        <QueryError error={failed.error} onRetry={failed.reload} />
      </section>
    );
  }
  if (providers.status === 'loading' || (editing && pool.status === 'loading')) {
    return <LoadingState label={t('common.state.loading')} />;
  }
  return (
    <UpstreamForm
      // Dipasang ulang saat data berganti supaya isian awal tidak di-set dari effect.
      key={poolId ?? 'baru'}
      client={client}
      providers={providers.data ?? []}
      initial={editing && pool.status === 'success' ? pool.data : null}
    />
  );
}
