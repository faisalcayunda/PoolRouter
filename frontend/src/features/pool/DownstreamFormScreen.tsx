import { useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { i18n as I18n } from 'i18next';
import {
  api,
  ApiError,
  type ApiClient,
  type DownstreamPool,
  type Provider,
  type UpstreamPool,
} from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { Switch } from '../../components/m3/Switch';
import { TextArea, TextField } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { useCrumbName } from '../common/CrumbName';
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
  JENIS_ALOKASI,
  serverErrors,
  SIFAT_ALOKASI,
  toDownstreamConfig,
  type DownstreamDraft,
  type DraftErrors,
  type DraftField,
  type FieldIssue,
} from './downstreamForm';

function tDynamic(instance: I18n, key: string, vars: Record<string, unknown> = {}): string {
  return (instance.t as (k: string, o?: Record<string, unknown>) => string)(key, vars);
}

function issueMessage(instance: I18n, issue: FieldIssue): string {
  // `alokasi_kelebihan` tanpa angka dari server memakai kalimat umum, bukan "sebanyak ".
  const kode =
    issue.kode === 'alokasi_kelebihan' && issue.batas === undefined
      ? 'alokasi_kelebihan_umum'
      : issue.kode;
  return tDynamic(instance, `downstream.form.error.${kode}`, { ...issue });
}

function focusField(field: DraftField): void {
  document
    .querySelector(`[data-field="${field}"]`)
    ?.querySelector<HTMLElement>('input, textarea, button')
    ?.focus();
}

interface DownstreamFormProps {
  client: ApiClient;
  upstreams: UpstreamPool[];
  providerNames: ReadonlyMap<string, string>;
  /** Pool yang sedang disunting; `null` = membuat baru. */
  initial: DownstreamPool | null;
}

function DownstreamForm({
  client,
  upstreams,
  providerNames,
  initial,
}: DownstreamFormProps): ReactNode {
  const { t, i18n } = useTranslation();
  const snackbar = useSnackbar();
  const titleRef = useFocusOnMount<HTMLHeadingElement>();
  const [current, setCurrent] = useState<DownstreamPool | null>(initial);
  useCrumbName(current?.downstream_pool_id, current?.nama);
  const [draft, setDraft] = useState<DownstreamDraft>(() =>
    initial ? draftFromPool(initial) : emptyDraft(),
  );
  const [errors, setErrors] = useState<DraftErrors>({});
  const [lainnya, setLainnya] = useState(0);
  const [failure, setFailure] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);

  const mode = current ? 'update' : 'create';
  useDocumentTitle(t('title.page.downstream'));

  function set<K extends DraftField>(field: K, value: DownstreamDraft[K]) {
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
    const built = toDownstreamConfig(draft, mode);
    if (!built.ok) {
      showErrors(built.errors);
      return;
    }
    setErrors({});
    setSaving(true);
    try {
      const saved = current
        ? await client.updateDownstreamPool(current.downstream_pool_id, built.body)
        : await client.createDownstreamPool(built.body);
      setCurrent(saved);
      setDraft(draftFromPool(saved));
      if (!current && typeof window !== 'undefined') {
        window.history.replaceState(
          null,
          '',
          `/downstream/${encodeURIComponent(saved.downstream_pool_id)}`,
        );
      }
      snackbar.notify(t('downstream.form.saved', { nama: saved.nama }));
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
  const upstreamLabel = (pool: UpstreamPool) =>
    providerNames.get(pool.provider_id) ?? pool.upstream_pool_id;
  const beralokasi = draft.jenis !== 'none';

  return (
    <section className="flex flex-col gap-4">
      <PageHeader
        title={
          current
            ? t('downstream.form.title_edit', { nama: current.nama })
            : t('downstream.page.add')
        }
        subtitle={t('downstream.page.subtitle')}
        titleRef={titleRef}
        actions={
          <LinkButton href="/downstream" variant="text">
            {t('downstream.form.back')}
          </LinkButton>
        }
      />

      {invalidCount > 0 ? (
        <InlineAlert tone="error">
          {t('downstream.form.error.summary', { jumlah: invalidCount })}
        </InlineAlert>
      ) : null}
      {lainnya > 0 ? (
        <InlineAlert tone="error">
          {t('downstream.form.error.lainnya', { jumlah: lainnya })}
        </InlineAlert>
      ) : null}
      {failure !== null ? (
        <InlineAlert tone="error">{errorMessage(t, i18n, failure).message}</InlineAlert>
      ) : null}

      <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-4">
        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">
            {mode === 'create'
              ? t('downstream.page.form_heading')
              : t('downstream.form.group.general')}
          </h2>
          <div className="grid gap-4 sm:grid-cols-2">
            <div data-field="nama">
              <TextField
                label={t('downstream.col.name')}
                value={draft.nama}
                onChange={(v) => set('nama', v)}
                autoComplete="off"
                {...err('nama')}
              />
            </div>
            <div>
              {/* Mode belum ada di kontrak API: kolom tampil seperti frame, tanpa nilai. */}
              <TextField
                label={t('downstream.col.mode')}
                value="–"
                isReadOnly
                isDisabled
                autoComplete="off"
              />
            </div>
            <div data-field="upstream_pool_id">
              <Select
                label={t('downstream.col.upstream')}
                selectedKey={draft.upstream_pool_id === '' ? null : draft.upstream_pool_id}
                onSelectionChange={(key) => {
                  const found = upstreams.find((p) => p.upstream_pool_id === key);
                  if (found) set('upstream_pool_id', found.upstream_pool_id);
                }}
                placeholder={t('downstream.form.upstream_placeholder')}
                isInvalid={errors.upstream_pool_id !== undefined}
              >
                {upstreams.map((p) => (
                  <SelectItem
                    key={p.upstream_pool_id}
                    id={p.upstream_pool_id}
                    textValue={upstreamLabel(p)}
                  >
                    {upstreamLabel(p)}
                  </SelectItem>
                ))}
              </Select>
              {errors.upstream_pool_id ? (
                <p className="typescale-body-small text-error">
                  {err('upstream_pool_id').errorMessage}
                </p>
              ) : null}
            </div>
            <div data-field="ttl_jam">
              <TextField
                label={t('downstream.page.ttl')}
                description={t('downstream.page.ttl_help')}
                value={draft.ttl_jam}
                onChange={(v) => set('ttl_jam', v)}
                inputMode="decimal"
                autoComplete="off"
                {...err('ttl_jam')}
              />
            </div>
          </div>
        </Card>

        <Card variant="outlined" className="flex flex-col gap-4">
          <h2 className="typescale-title-medium">{t('downstream.form.group.alokasi')}</h2>
          <div data-field="jenis">
            <Select
              label={t('downstream.form.alokasi.jenis')}
              selectedKey={draft.jenis}
              onSelectionChange={(key) => {
                const found = JENIS_ALOKASI.find((v) => v === key);
                if (!found) return;
                set('jenis', found);
                // Tanpa alokasi, kontrak mewajibkan boleh_pinjam true dan tanpa sifat.
                if (found === 'none') {
                  set('sifat', null);
                  set('nilai', '');
                  set('boleh_pinjam', true);
                }
              }}
            >
              {JENIS_ALOKASI.map((v) => (
                <SelectItem key={v} id={v}>
                  {tDynamic(i18n, `downstream.form.alokasi.jenis_${v}`)}
                </SelectItem>
              ))}
            </Select>
          </div>
          {beralokasi ? (
            <>
              <div data-field="nilai">
                <TextField
                  label={
                    draft.jenis === 'tetap'
                      ? t('downstream.form.alokasi.nilai_tetap')
                      : t('downstream.form.alokasi.nilai_persen')
                  }
                  value={draft.nilai}
                  onChange={(v) => set('nilai', v)}
                  inputMode="decimal"
                  autoComplete="off"
                  {...err('nilai')}
                />
              </div>
              <div data-field="sifat">
                <Select
                  label={t('downstream.form.alokasi.sifat')}
                  selectedKey={draft.sifat}
                  placeholder={t('downstream.form.alokasi.sifat_placeholder')}
                  onSelectionChange={(key) => {
                    const found = SIFAT_ALOKASI.find((v) => v === key);
                    if (found) set('sifat', found);
                  }}
                  isInvalid={errors.sifat !== undefined}
                >
                  {SIFAT_ALOKASI.map((v) => (
                    <SelectItem key={v} id={v}>
                      {tDynamic(i18n, `downstream.form.alokasi.sifat_${v}`)}
                    </SelectItem>
                  ))}
                </Select>
                {errors.sifat ? (
                  <p className="typescale-body-small text-error">{err('sifat').errorMessage}</p>
                ) : null}
              </div>
              <div data-field="boleh_pinjam">
                <Switch isSelected={draft.boleh_pinjam} onChange={(v) => set('boleh_pinjam', v)}>
                  {t('downstream.form.alokasi.boleh_pinjam')}
                </Switch>
              </div>
            </>
          ) : (
            <p className="typescale-body-medium text-on-surface-variant">
              {t('downstream.form.alokasi.none_help')}
            </p>
          )}
        </Card>

        {mode === 'update' ? (
          <Card variant="outlined" className="flex flex-col gap-4">
            <h2 className="typescale-title-medium">{t('downstream.form.reason.heading')}</h2>
            <div data-field="alasan">
              <TextArea
                label={t('downstream.form.reason.label')}
                description={t('downstream.form.reason.help', {
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
          <LinkButton href="/downstream" variant="text">
            {t('common.action.cancel')}
          </LinkButton>
          <Button type="submit" isDisabled={saving}>
            {saving
              ? t('provider.form.saving')
              : mode === 'update'
                ? t('downstream.form.save_edit')
                : t('downstream.form.save_new')}
          </Button>
        </div>
      </form>
      <SnackbarRegion queue={snackbar.queue} />
    </section>
  );
}

export interface DownstreamFormScreenProps {
  client?: ApiClient;
  /** `undefined` = downstream pool baru (`/downstream/baru`). */
  poolId?: string;
}

/** S07 form downstream pool: buat baru atau sunting konfigurasi saat ini (ARC-102). */
export function DownstreamFormScreen({ client = api, poolId }: DownstreamFormScreenProps) {
  const { t } = useTranslation();
  const role = useRole();
  const allowed = role === null || role === 'platform_admin';
  const editing = poolId !== undefined;
  const upstreams = useQuery<UpstreamPool[]>(
    (signal) => client.upstreamPools({ signal }),
    'upstream',
    { enabled: allowed },
  );
  const providers = useQuery<Provider[]>(
    (signal) => client.listProviders({ signal }),
    'providers',
    {
      enabled: allowed,
    },
  );
  const pool = useQuery<DownstreamPool>(
    (signal) => client.getDownstreamPool(poolId ?? '', { signal }),
    `downstream:${poolId ?? ''}`,
    { enabled: allowed && editing },
  );

  if (!allowed) return <Forbidden />;
  const failed = upstreams.status === 'error' ? upstreams : pool.status === 'error' ? pool : null;
  if (failed) {
    return (
      <section className="flex flex-col gap-4">
        <PageHeader title={t('downstream.page.title')} />
        <QueryError error={failed.error} onRetry={failed.reload} />
      </section>
    );
  }
  // Daftar provider hanya memberi nama pada pilihan upstream; gagal membacanya tidak menghalangi form.
  if (
    upstreams.status === 'loading' ||
    providers.status === 'loading' ||
    (editing && pool.status === 'loading')
  ) {
    return <LoadingState label={t('common.state.loading')} />;
  }
  const names = new Map(
    (providers.status === 'success' ? providers.data : []).map((p) => [p.provider_id, p.nama]),
  );
  return (
    <DownstreamForm
      key={poolId ?? 'baru'}
      client={client}
      upstreams={upstreams.data ?? []}
      providerNames={names}
      initial={editing && pool.status === 'success' ? pool.data : null}
    />
  );
}
