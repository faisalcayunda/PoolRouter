import { useEffect, useRef, useState, type FormEvent, type Key, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { i18n as I18n } from 'i18next';
import { api, ApiError, type ApiClient, type Endpoint, type Provider } from '../../api/client';
import { Button } from '../../components/m3/Button';
import { Card } from '../../components/m3/Card';
import { InlineAlert } from '../../components/m3/InlineAlert';
import { Select, SelectItem } from '../../components/m3/Select';
import { StatusLabel } from '../../components/m3/StatusLabel';
import { Switch } from '../../components/m3/Switch';
import { Tab, TabList, TabPanel, Tabs } from '../../components/m3/Tabs';
import { TextArea, TextField } from '../../components/m3/TextField';
import { useDocumentTitle, useFocusOnMount } from '../common/a11y';
import { useCrumbName } from '../common/CrumbName';
import { errorMessage, LoadingState, PageHeader, QueryError } from '../common/page';
import { useRole } from '../common/RoleContext';
import { useQuery } from '../common/useQuery';
import { SnackbarRegion, useSnackbar } from '../common/useSnackbar';
import { Forbidden } from './Forbidden';
import { FieldGrid, SectionCard, SELECT_FILL } from '../common/layout';
import { FormSection } from './FormSection';
import { LinkButton } from './LinkButton';
import { RuleDialog } from './RuleDialog';
import { TemplateEditor } from './TemplateEditor';
import {
  ALASAN_MAKS,
  CONFIG_SCHEMA_VERSIONS,
  draftFromProvider,
  emptyDraft,
  firstInvalid,
  LOG_MODE,
  METODE,
  parsePins,
  serverErrors,
  TLS_TRUST,
  toProviderConfig,
  type DraftErrors,
  type DraftField,
  type FieldIssue,
  type ProviderDraft,
} from './providerForm';
import { parseParameters, serializeParameters, type ParameterRule } from './ruleEditor';

/** Tab detail provider (Penpot F0 / S03a, S03, S04, S03b, S04b); urutan = urutan di layar. */
const TABS = ['umum', 'template', 'parameter', 'respons', 'endpoint'] as const;
type ProviderTab = (typeof TABS)[number];

const TAB_LABEL: Record<ProviderTab, string> = {
  umum: 'provider.tab.general',
  template: 'provider.tab.template',
  parameter: 'provider.tab.rules',
  respons: 'provider.tab.schema',
  endpoint: 'provider.tab.endpoint',
};

const TAB_SUBTITLE: Record<ProviderTab, string> = {
  umum: 'provider.subtitle.general',
  template: 'provider.subtitle.template',
  parameter: 'provider.subtitle.rules',
  respons: 'provider.subtitle.schema',
  endpoint: 'provider.subtitle.endpoint',
};

/** Tab yang memuat tiap field draft, supaya galat membuka tab yang benar sebelum fokus. */
const FIELD_TAB: Record<DraftField, ProviderTab> = {
  nama: 'umum',
  base_url: 'umum',
  metode: 'umum',
  tls_trust: 'umum',
  tls_ca_pem: 'umum',
  tls_pin_spki: 'umum',
  log_mode: 'umum',
  config_schema_version: 'umum',
  alasan: 'umum',
  body_template: 'template',
  parameters: 'parameter',
  respons_skema: 'respons',
  data_balikan: 'respons',
};

const SAVE_LABEL: Record<ProviderTab, string> = {
  umum: 'provider.form.save_edit',
  template: 'provider.template.save',
  parameter: 'provider.rules.save',
  respons: 'provider.form.save_edit',
  endpoint: 'provider.form.save_edit',
};

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
  (
    box?.querySelector<HTMLElement>('input, textarea, button') ?? (box as HTMLElement | null)
  )?.focus();
}

type RuleRow = ParameterRule;

const RULE_TYPE_KEY = {
  regex: 'provider.rules.type.regex',
  enum: 'provider.rules.type.enum',
  panjang: 'provider.rules.type.length',
  format_tanggal: 'provider.rules.type.date_format',
} as const;

function ruleTypeLabel(instance: I18n, rule: RuleRow): string {
  const jenis = rule.rule?.jenis;
  return jenis ? tDynamic(instance, RULE_TYPE_KEY[jenis]) : '-';
}

/** Ringkasan nilai rule; `null` = belum ditetapkan. */
function ruleValueLabel(instance: I18n, rule: RuleRow): string | null {
  const r = rule.rule;
  if (!r) return null;
  switch (r.jenis) {
    case 'regex':
      return r.pola ?? null;
    case 'enum':
      return r.nilai_diizinkan?.length ? r.nilai_diizinkan.join(', ') : null;
    case 'format_tanggal':
      return r.format_tanggal ?? null;
    case 'panjang':
      return r.panjang_min != null || r.panjang_maks != null
        ? tDynamic(instance, 'provider.rules.value.length', {
            min: r.panjang_min ?? 0,
            maks: r.panjang_maks ?? '∞',
          })
        : null;
  }
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
  const [tab, setTab] = useState<ProviderTab>('umum');
  const pendingFocus = useRef<DraftField | null>(null);
  /** Editor rule terbuka: `index` null = parameter baru. */
  const [ruleDialog, setRuleDialog] = useState<{ index: number | null } | null>(null);

  const mode = current ? 'update' : 'create';
  useDocumentTitle(t('title.page.provider'));
  useCrumbName(current?.provider_id, current?.nama);

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

  function saveRule(row: ParameterRule) {
    const list = [...(parseParameters(draft.parameters) ?? [])];
    const index = ruleDialog?.index ?? null;
    if (index === null) list.push(row);
    else list[index] = row;
    set('parameters', serializeParameters(list));
    setRuleDialog(null);
  }

  function removeRule(index: number) {
    const list = (parseParameters(draft.parameters) ?? []).filter((_, i) => i !== index);
    set('parameters', serializeParameters(list));
  }

  function showErrors(next: DraftErrors) {
    setErrors(next);
    const first = firstInvalid(next);
    if (first) {
      // Buka tab pemilik field dulu; fokus dipindah setelah panelnya aktif (efek di bawah).
      pendingFocus.current = first;
      setTab(FIELD_TAB[first]);
    }
  }

  useEffect(() => {
    const field = pendingFocus.current;
    if (field === null) return;
    pendingFocus.current = null;
    focusField(field);
  });

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
  // Versi 2 disembunyikan untuk provider baru sampai layar S03 dengan editor rule terstruktur dan
  // data balikan ada (ARC-242, sisa S03a/S03b/S04). Provider yang tersimpan berversi 2 tetap melihatnya,
  // walau draf sempat dipindah ke versi 1, supaya penurunan tidak tak bisa dibatalkan dari form.
  const versiTersedia = CONFIG_SCHEMA_VERSIONS.filter(
    (v) => v === 1 || current?.config_schema_version === v,
  );
  const showDataBalikan = draft.config_schema_version === 2 || draft.data_balikan.trim() !== '';
  const templateIssue = errors.body_template;
  const rules = parseParameters(draft.parameters);
  const saveLabel = saving
    ? t('provider.form.saving')
    : mode === 'create'
      ? t('provider.form.save_new')
      : tDynamic(i18n, SAVE_LABEL[tab]);

  return (
    <form noValidate onSubmit={(e) => void submit(e)} className="flex flex-col gap-6">
      <PageHeader
        title={current ? current.nama : t('provider.page.add')}
        subtitle={tDynamic(i18n, TAB_SUBTITLE[tab])}
        titleRef={titleRef}
        actions={
          tab === 'parameter' ? (
            <Button type="button" onPress={() => setRuleDialog({ index: null })}>
              {t('provider.rules.add')}
            </Button>
          ) : (
            <>
              <LinkButton href="/provider" variant="outlined">
                {t('common.action.cancel')}
              </LinkButton>
              <Button type="submit" isDisabled={saving}>
                {saveLabel}
              </Button>
            </>
          )
        }
      />

      {templateIssue ? (
        <InlineAlert tone="error" heading={t('provider.template.error.title')}>
          {issueMessage(i18n, templateIssue)}
          {templateIssue.kode === 'template_secret_tidak_ada' && templateIssue.nama ? (
            // Tab baru: template yang belum disimpan tetap ada di layar ini.
            <LinkButton href="/secret" variant="text" newTab className="mt-2">
              {t('provider.template.error.secret_create', { nama: templateIssue.nama })}
            </LinkButton>
          ) : null}
        </InlineAlert>
      ) : null}
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

      <Tabs
        selectedKey={tab}
        onSelectionChange={(key: Key) => {
          const found = TABS.find((v) => v === key);
          if (found) setTab(found);
        }}
      >
        <TabList aria-label={t('provider.tab.aria')}>
          {TABS.map((id) => (
            <Tab key={id} id={id}>
              {tDynamic(i18n, TAB_LABEL[id])}
            </Tab>
          ))}
        </TabList>

        <TabPanel id="umum" shouldForceMount className="pt-4">
          <Card variant="container">
            <FormSection
              title={t('provider.general.identity.title')}
              help={t('provider.general.identity.help')}
            >
              <FieldGrid>
                <div data-field="nama">
                  <TextField
                    label={t('provider.general.name')}
                    value={draft.nama}
                    onChange={(v) => set('nama', v)}
                    autoComplete="off"
                    {...err('nama')}
                  />
                </div>
                <div data-field="base_url" className="medium:col-span-2">
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
              </FieldGrid>
            </FormSection>

            <FormSection
              title={t('provider.general.method_auth.title')}
              help={t('provider.general.auth.help')}
            >
              <FieldGrid>
                <div data-field="metode">
                  <Select
                    className={SELECT_FILL}
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
                {/* Pilihan autentikasi belum ada di kontrak API: tampil seperti di Penpot, tidak bisa diubah. */}
                <div className="flex flex-col gap-1">
                  <Select
                    className={SELECT_FILL}
                    label={t('provider.general.auth.label')}
                    selectedKey="body"
                    isDisabled
                  >
                    <SelectItem id="body" textValue={t('provider.general.auth.body')}>
                      {t('provider.general.auth.body')}
                    </SelectItem>
                  </Select>
                  <p className="typescale-body-small text-on-surface-variant">
                    {t('provider.api_pending')}
                  </p>
                </div>
              </FieldGrid>
            </FormSection>

            <FormSection
              title={t('provider.general.security.title')}
              help={t('provider.general.security.help')}
            >
              <FieldGrid>
                <div data-field="tls_trust">
                  <Select
                    className={SELECT_FILL}
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
                {draft.tls_trust === 'ca_kustom' ? (
                  <div data-field="tls_ca_pem" className="col-span-full">
                    <TextArea
                      label={t('provider.form.tls_ca_pem.label')}
                      description={t('provider.form.tls_ca_pem.help')}
                      value={draft.tls_ca_pem}
                      onChange={(v) => set('tls_ca_pem', v)}
                      rows={8}
                      textAreaClassName="font-code"
                      {...err('tls_ca_pem')}
                    />
                  </div>
                ) : null}
                {draft.tls_trust === 'pin_spki' ? (
                  <div data-field="tls_pin_spki" className="col-span-full flex flex-col gap-3">
                    <TextArea
                      label={t('provider.form.tls_pin_spki.label')}
                      description={t('provider.form.tls_pin_spki.help')}
                      value={draft.tls_pin_spki}
                      onChange={(v) => set('tls_pin_spki', v)}
                      rows={4}
                      textAreaClassName="font-code"
                      {...err('tls_pin_spki')}
                    />
                    {parsePins(draft.tls_pin_spki).length === 1 ? (
                      <InlineAlert tone="warning">
                        {t('provider.form.tls_pin_spki.single')}
                      </InlineAlert>
                    ) : null}
                  </div>
                ) : null}
                <div data-field="log_mode">
                  <Select
                    className={SELECT_FILL}
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
                    className={SELECT_FILL}
                    label={t('provider.form.version.label')}
                    selectedKey={draft.config_schema_version}
                    onSelectionChange={(key) => {
                      const found = versiTersedia.find((v) => v === key);
                      if (found !== undefined) set('config_schema_version', found);
                    }}
                  >
                    {versiTersedia.map((v) => (
                      <SelectItem key={v} id={v}>
                        {tDynamic(i18n, `provider.form.version.v${v}`)}
                      </SelectItem>
                    ))}
                  </Select>
                </div>
              </FieldGrid>
            </FormSection>

            <FormSection
              title={t('provider.general.flag.title')}
              help={t('provider.general.flag.help')}
            >
              {/* Penanda blend belum ada di kontrak API: saklar tampil nonaktif. */}
              <Switch isDisabled>{t('provider.general.flag.switch')}</Switch>
              <p className="typescale-body-medium text-on-surface-variant">
                {`${t('provider.general.flag.note')} (${t('provider.api_pending')})`}
              </p>
            </FormSection>

            {current ? (
              <FormSection
                title={t('provider.general.status.title')}
                help={t('provider.general.status.help')}
              >
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <StatusLabel tone={current.aktif ? 'success' : 'neutral'}>
                    {current.aktif ? t('common.state.active') : t('provider.list.inactive')}
                  </StatusLabel>
                  {/* Belum ada operasi nonaktifkan provider di API. */}
                  <Button variant="outlined" isDisabled>
                    {t('provider.page.deactivate')}
                  </Button>
                </div>
              </FormSection>
            ) : null}

            {mode === 'update' ? (
              <FormSection
                title={t('provider.form.reason.heading')}
                help={t('provider.form.reason.help', {
                  jumlah: draft.alasan.trim().length,
                  batas: ALASAN_MAKS,
                })}
              >
                <div data-field="alasan">
                  <TextArea
                    label={t('provider.form.reason.label')}
                    value={draft.alasan}
                    onChange={(v) => set('alasan', v)}
                    autoComplete="off"
                    rows={3}
                    {...err('alasan')}
                  />
                </div>
              </FormSection>
            ) : null}
          </Card>
        </TabPanel>

        <TabPanel id="template" shouldForceMount className="pt-4">
          <SectionCard title={t('provider.template.title')}>
            <div data-field="body_template">
              <TemplateEditor
                label={t('provider.template.label')}
                lineNote={
                  templateIssue?.posisi
                    ? t('provider.template.error.line', { baris: templateIssue.posisi.baris })
                    : undefined
                }
                description={t('provider.template.help', { contoh: '{{variabel}}' })}
                value={draft.body_template}
                onChange={(v) => set('body_template', v)}
                isInvalid={templateIssue !== undefined}
              />
            </div>
          </SectionCard>
        </TabPanel>

        <TabPanel id="parameter" shouldForceMount className="pt-4">
          <div className="flex flex-col gap-6">
            <InlineAlert tone="info" heading={t('provider.rules.info_title')}>
              {t('provider.rules.info_body')}
            </InlineAlert>
            <Card variant="container">
              {rules === null || rules.length === 0 ? (
                <p className="typescale-body-medium p-6 text-on-surface-variant">
                  {t('provider.rules.empty.title')}
                </p>
              ) : (
                <div className="table-card">
                  <table className="w-full border-collapse text-left">
                    <caption className="sr-only">{t('provider.rules.caption')}</caption>
                    <thead>
                      <tr className="typescale-label-small text-on-surface-variant">
                        <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                          {t('provider.rules.col.param')}
                        </th>
                        <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                          {t('provider.rules.col.type')}
                        </th>
                        <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                          {t('provider.rules.col.value')}
                        </th>
                        <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                          {t('provider.rules.col.status')}
                        </th>
                        <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                          {t('provider.rules.col.action')}
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {rules.map((rule, index) => (
                        <tr
                          key={`${rule.nama}-${index}`}
                          className="border-b border-outline-variant text-on-surface last:border-b-0"
                        >
                          <td
                            data-label={t('provider.rules.col.param')}
                            className="typescale-title-small px-5 py-4 align-middle"
                          >
                            {rule.nama}
                          </td>
                          <td
                            data-label={t('provider.rules.col.type')}
                            className="typescale-body-medium px-5 py-4 align-middle"
                          >
                            {ruleTypeLabel(i18n, rule)}
                          </td>
                          <td
                            data-label={t('provider.rules.col.value')}
                            className="typescale-body-medium px-5 py-4 align-middle text-on-surface-variant"
                          >
                            {ruleValueLabel(i18n, rule) ?? t('provider.rules.not_set')}
                          </td>
                          <td
                            data-label={t('provider.rules.col.status')}
                            className="typescale-body-medium px-5 py-4 align-middle text-on-surface-variant"
                          >
                            {t('provider.api_pending')}
                          </td>
                          <td
                            data-label={t('provider.rules.col.action')}
                            className="px-2 py-2 align-middle"
                          >
                            <Button
                              variant="text"
                              aria-label={t('provider.rules.edit_aria', { nama: rule.nama })}
                              onPress={() => setRuleDialog({ index })}
                            >
                              {t('common.action.edit')}
                            </Button>
                            <Button
                              variant="text"
                              aria-label={t('provider.rules.delete_aria', { nama: rule.nama })}
                              onPress={() => removeRule(index)}
                            >
                              {t('common.action.delete')}
                            </Button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </Card>
            <Card variant="container" className="flex flex-col gap-4 p-6">
              <div className="flex flex-col gap-1">
                <h2 className="typescale-title-large text-primary-text">
                  {t('provider.rules.panel_title')}
                </h2>
                <p className="typescale-body-medium text-on-surface-variant">
                  {t('provider.rules.type_help')}
                </p>
              </div>
              {errors.parameters ? (
                <div data-field="parameters" tabIndex={-1} className="m3-focus">
                  <InlineAlert tone="error" heading={t('provider.rules.error_title')}>
                    {issueMessage(i18n, errors.parameters)}
                  </InlineAlert>
                </div>
              ) : null}
              <p className="typescale-body-medium text-on-surface-variant">
                {t('provider.rules.no_defaults')}
              </p>
              {tab === 'parameter' ? (
                <div className="flex flex-wrap items-center justify-end gap-2">
                  <LinkButton href="/provider" variant="outlined">
                    {t('common.action.cancel')}
                  </LinkButton>
                  <Button type="submit" isDisabled={saving}>
                    {saveLabel}
                  </Button>
                </div>
              ) : null}
            </Card>
          </div>
        </TabPanel>

        <TabPanel id="respons" shouldForceMount className="pt-4">
          <div className="flex flex-col gap-6">
            <Card variant="container" className="p-6">
              <div className="flex flex-col gap-3 rounded-large border border-outline-variant bg-surface p-4">
                <div data-field="respons_skema">
                  <TextArea
                    label={t('provider.schema.label')}
                    description={t('provider.schema.help')}
                    value={draft.respons_skema}
                    onChange={(v) => set('respons_skema', v)}
                    rows={6}
                    textAreaClassName="font-code"
                    {...err('respons_skema')}
                  />
                </div>
              </div>
            </Card>

            {showDataBalikan ? (
              <Card variant="container" className="flex flex-col gap-4 p-6">
                <h2 className="typescale-title-large text-primary-text">
                  {t('provider.form.data_balikan.heading')}
                </h2>
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

            <div className="flex flex-wrap items-center justify-between gap-3">
              <h2 className="typescale-title-large text-primary-text">
                {t('provider.schema.table_title')}
              </h2>
              {/* Kode respons belum ada di kontrak API: tombol tampil nonaktif. */}
              <Button isDisabled>{t('provider.schema.add_code')}</Button>
            </div>
            <Card variant="container">
              <div className="table-card">
                <table className="w-full border-collapse text-left">
                  <caption className="sr-only">{t('provider.schema.table_title')}</caption>
                  <thead>
                    <tr className="typescale-label-small text-on-surface-variant">
                      <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                        {t('provider.schema.col.code_full')}
                      </th>
                      <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                        {t('provider.schema.col.status')}
                      </th>
                      <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                        {t('provider.schema.col.middleware')}
                      </th>
                      <th scope="col" className="border-b border-outline-variant px-5 py-3.5">
                        {t('provider.schema.col.category')}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td
                        colSpan={4}
                        className="typescale-body-medium px-5 py-6 text-on-surface-variant"
                      >
                        {t('provider.api_pending')}
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </Card>
          </div>
        </TabPanel>

        <TabPanel id="endpoint" shouldForceMount className="pt-4">
          <EndpointTab
            client={client}
            provider={current}
            enabled={current !== null && tab === 'endpoint'}
          />
        </TabPanel>
      </Tabs>
      <SnackbarRegion queue={snackbar.queue} />
      {ruleDialog ? (
        <RuleDialog
          initial={ruleDialog.index === null ? null : (rules?.[ruleDialog.index] ?? null)}
          editingIndex={ruleDialog.index}
          existingNames={(rules ?? []).map((rule) => rule.nama)}
          onSave={saveRule}
          onCancel={() => setRuleDialog(null)}
        />
      ) : null}
    </form>
  );
}

/** Tab Endpoint (S04b): pemberitahuan tingkat admin dan parameter konstan endpoint milik provider ini. */
function EndpointTab({
  client,
  provider,
  enabled,
}: {
  client: ApiClient;
  provider: Provider | null;
  enabled: boolean;
}): ReactNode {
  const { t } = useTranslation();
  const providerId = provider?.provider_id ?? '';
  const endpoints = useQuery<Endpoint[]>(
    async (signal) =>
      (await client.listEndpoints({ signal })).filter((e) => e.provider_id === providerId),
    `provider-endpoints:${providerId}`,
    { enabled },
  );
  return (
    <div className="flex flex-col gap-6">
      <InlineAlert tone="info" heading={t('endpoint.form.notice.title')}>
        {t('endpoint.form.notice.body')}
      </InlineAlert>
      {endpoints.status === 'loading' ? (
        <LoadingState label={t('common.state.loading')} />
      ) : endpoints.status === 'error' ? (
        <QueryError error={endpoints.error} onRetry={endpoints.reload} />
      ) : endpoints.data.length === 0 ? (
        <Card variant="container" className="flex flex-col items-start gap-4 p-6">
          <p className="typescale-body-medium text-on-surface-variant">
            {t('provider.endpoint_tab.empty')}
          </p>
          <LinkButton href="/endpoint" variant="outlined">
            {t('provider.endpoint_tab.open')}
          </LinkButton>
        </Card>
      ) : (
        endpoints.data.map((endpoint) => (
          <Card
            key={endpoint.endpoint_id}
            variant="container"
            className="flex flex-col items-start gap-4 p-6"
          >
            <h2 className="typescale-title-large text-primary-text">
              {`${t('endpoint.form.constants.title')} ${provider?.nama ?? ''}`}
            </h2>
            <TextField
              label="TRESHOLD"
              value={endpoint.parameter_literal['TRESHOLD'] ?? ''}
              isReadOnly
              autoComplete="off"
              description={t('provider.endpoint_tab.threshold_help')}
            />
            <LinkButton
              href={`/endpoint/${encodeURIComponent(endpoint.endpoint_id)}`}
              variant="outlined"
            >
              {t('provider.endpoint_tab.edit')}
            </LinkButton>
          </Card>
        ))
      )}
    </div>
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
      <section className="flex flex-col gap-6">
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
