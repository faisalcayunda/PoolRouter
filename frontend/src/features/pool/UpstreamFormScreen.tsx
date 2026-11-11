import { useId, useState, type FormEvent, type ReactNode } from 'react';
import { flushSync } from 'react-dom';
import { useTranslation } from 'react-i18next';
import type { i18n as I18n } from 'i18next';
import {
  api,
  ApiError,
  type ApiClient,
  type HasilK20,
  type Provider,
  type UpstreamPool,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { Tab, TabList, TabPanel, Tabs } from '../../components/m3/Tabs';
import { TextArea, TextField } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { useCrumbName } from '../common/CrumbName';
import { FieldGrid, SectionCard, SELECT_FILL } from '../common/layout';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from '../provider/Forbidden';
import { LinkButton } from '../provider/LinkButton';
import { NonaktifkanDialog } from './NonaktifkanDialog';
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

type TabId = 'quota' | 'antipattern';

/** Field tab Kuota dan laju (S06), urutan sama dengan frame. */
const QUOTA_FIELDS: NumberField[] = ['kuota_harian', 'rate_limit_per_jam', 'porsi_sync_persen'];

/** Field tab Anti-pola (S06b), urutan sama dengan frame. */
const ANTIPATTERN_FIELDS: NumberField[] = ['jeda_min', 'jeda_maks', 'ambang_berdampingan'];

/** Kartu tambahan di tab Kuota dan laju untuk field kontrak yang tidak digambar di frame. */
const GROUPS: { heading: string; fields: NumberField[] }[] = [
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

/** Tab yang memuat field itu; `null` untuk field di luar tab (alasan). */
function tabOf(field: DraftField): TabId | null {
  if (field === 'alasan') return null;
  return (ANTIPATTERN_FIELDS as readonly string[]).includes(field) ? 'antipattern' : 'quota';
}

interface UpstreamFormProps {
  client: ApiClient;
  providers: Provider[];
  /** Pool yang sedang disunting; `null` = membuat baru. */
  initial: UpstreamPool | null;
}

function UpstreamForm({ client, providers, initial }: UpstreamFormProps): ReactNode {
  const { t, i18n } = useTranslation();
  const statusLabelId = useId();
  const snackbar = useSnackbar();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const [current, setCurrent] = useState<UpstreamPool | null>(initial);
  useCrumbName(
    current?.upstream_pool_id,
    providers.find((p) => p.provider_id === current?.provider_id)?.nama,
  );
  const [draft, setDraft] = useState<UpstreamDraft>(() =>
    initial ? draftFromPool(initial) : emptyDraft(),
  );
  const [errors, setErrors] = useState<DraftErrors>({});
  const [lainnya, setLainnya] = useState(0);
  const [tab, setTab] = useState<TabId>('quota');
  const [deactivating, setDeactivating] = useState(false);
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
    const first = firstInvalid(next);
    const target = first ? tabOf(first) : null;
    // Panel tab yang tidak aktif tidak terpasang: pindah tab dulu, baru fokus ke field salah.
    flushSync(() => {
      setErrors(next);
      if (target) setTab(target);
    });
    if (first) focusField(first);
  }

  function deactivated(pool: UpstreamPool, hasil: HasilK20) {
    setDeactivating(false);
    setCurrent({ ...pool, aktif: false });
    snackbar.notify(
      t('upstream.deactivate.done', {
        nama: providerName(pool.provider_id),
        dibatalkan: hasil.jumlah_cancelled,
        gagal: hasil.jumlah_failed,
      }),
    );
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
  const rateLimit = Number(draft.rate_limit_per_jam);

  return (
    <section className="flex flex-col gap-6">
      <PageHeader
        title={
          current
            ? t('upstream.form.title_edit', { nama: providerName(current.provider_id) })
            : t('upstream.page.add')
        }
        titleRef={titleRef}
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

      <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-6">
        <Tabs
          selectedKey={tab}
          onSelectionChange={(key) => setTab(key === 'antipattern' ? 'antipattern' : 'quota')}
        >
          <TabList aria-label={t('upstream.tab.aria')}>
            <Tab id="quota">{t('upstream.tab.quota')}</Tab>
            <Tab id="antipattern">{t('upstream.tab.antipattern')}</Tab>
          </TabList>

          <TabPanel id="quota" className="flex flex-col gap-6">
            <SectionCard>
              <FieldGrid>
                <div data-field="provider_id">
                  <Select
                    className={SELECT_FILL}
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
                    <p className="typescale-body-small text-error">
                      {err('provider_id').errorMessage}
                    </p>
                  ) : (
                    <p className="typescale-body-small text-on-surface-variant">
                      {t('upstream.provider.help')}
                    </p>
                  )}
                </div>
                {current ? (
                  <div className="flex flex-col gap-1">
                    <span
                      id={statusLabelId}
                      className="typescale-body-small text-on-surface-variant"
                    >
                      {t('upstream.page.status')}
                    </span>
                    <div
                      role="group"
                      aria-labelledby={statusLabelId}
                      className="flex h-14 items-center"
                    >
                      <StatusLabel tone={current.aktif ? 'success' : 'neutral'}>
                        {current.aktif ? t('common.state.active') : t('upstream.status.inactive')}
                      </StatusLabel>
                    </div>
                  </div>
                ) : null}
                {QUOTA_FIELDS.map((field) => (
                  <div key={field} data-field={field}>
                    <TextField
                      label={tDynamic(i18n, `upstream.form.field.${field}`)}
                      description={
                        field === 'kuota_harian'
                          ? t('upstream.daily_quota.help')
                          : field === 'porsi_sync_persen'
                            ? t('upstream.sync_share.help')
                            : undefined
                      }
                      value={draft[field]}
                      onChange={(v) => set(field, v)}
                      inputMode="decimal"
                      autoComplete="off"
                      {...err(field)}
                    />
                  </div>
                ))}
                <div data-field="jendela_kuota">
                  <Select
                    className={SELECT_FILL}
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
              </FieldGrid>
            </SectionCard>

            {GROUPS.map((group) => (
              <SectionCard key={group.heading} title={tDynamic(i18n, group.heading)}>
                <FieldGrid>
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
                </FieldGrid>
              </SectionCard>
            ))}
          </TabPanel>

          <TabPanel id="antipattern" className="flex flex-col gap-6">
            <SectionCard>
              <p className="typescale-body-medium text-on-surface-variant">
                {t('antipattern.page.subtitle')}
              </p>
              <FieldGrid>
                {ANTIPATTERN_FIELDS.map((field) => (
                  <div key={field} data-field={field}>
                    <TextField
                      label={tDynamic(i18n, `upstream.form.field.${field}`)}
                      description={
                        field === 'jeda_min'
                          ? t('antipattern.page.delay_help')
                          : field === 'ambang_berdampingan'
                            ? t('antipattern.page.adjacent_help')
                            : undefined
                      }
                      value={draft[field]}
                      onChange={(v) => set(field, v)}
                      inputMode="decimal"
                      autoComplete="off"
                      {...err(field)}
                    />
                  </div>
                ))}
              </FieldGrid>
              {rateLimit > 0 ? (
                <InlineAlert tone="warning">
                  {t('antipattern.page.warn', { batas: rateLimit })}
                </InlineAlert>
              ) : null}
            </SectionCard>
          </TabPanel>
        </Tabs>

        <SectionCard title={t('upstream.form.reason.heading')}>
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
        </SectionCard>

        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" isDisabled={saving}>
            {saving ? t('provider.form.saving') : t('common.action.save')}
          </Button>
          <LinkButton href="/upstream" variant="text">
            {t('common.action.cancel')}
          </LinkButton>
          {current?.aktif ? (
            <Button
              variant="outlined"
              className="medium:ml-auto"
              onPress={() => setDeactivating(true)}
            >
              {t('upstream.page.deactivate')}
            </Button>
          ) : null}
        </div>
      </form>

      {deactivating && current ? (
        <NonaktifkanDialog
          title={t('upstream.deactivate.title')}
          tujuan={providerName(current.provider_id)}
          loadPreview={(signal) =>
            client.pratinjauNonaktifUpstreamPool(current.upstream_pool_id, { signal })
          }
          onConfirm={(alasan) => client.nonaktifkanUpstreamPool(current.upstream_pool_id, alasan)}
          onClose={() => setDeactivating(false)}
          onDone={(hasil) => deactivated(current, hasil)}
        />
      ) : null}
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}

export interface UpstreamFormScreenProps {
  client?: ApiClient;
  /** `undefined` = upstream pool baru (`/upstream/baru`). */
  poolId?: string;
}

/** S06 + S06b form upstream pool: buat baru atau sunting konfigurasi saat ini (ARC-102, ARC-247). */
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
      <section className="flex flex-col gap-6">
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
