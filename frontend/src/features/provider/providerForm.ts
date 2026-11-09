import type { Problem, Provider, ProviderConfig, Schemas } from '../../api/client';

/**
 * Model form provider (S03–S04, ARC-101). `PUT /providers/{id}` adalah penggantian penuh (V-03):
 * field yang tidak dikirim hilang dari versi baru. Karena itu form menyimpan SEMUA field
 * `ProviderConfig`, termasuk `data_balikan`, dan badan request disusun ulang dari draft setiap kali
 * disimpan, bukan dari objek `Provider` yang dibaca (yang memuat `provider_id`, `aktif`, dan versi).
 */

export type MetodeProvider = Schemas['MetodeProvider'];
export type TlsTrust = Schemas['TlsTrust'];
export type LogMode = Schemas['LogMode'];
export type Posisi = Schemas['Posisi'];

export const METODE: readonly MetodeProvider[] = ['POST', 'GET'];
export const TLS_TRUST: readonly TlsTrust[] = ['system', 'ca_kustom', 'pin_spki'];
export const LOG_MODE: readonly LogMode[] = ['metadata_only', 'full_payload'];
export const CONFIG_SCHEMA_VERSIONS = [1, 2] as const;

/** Batas `ProviderConfig.alasan` di kontrak (minLength 10, maxLength 500). */
export const ALASAN_MIN = 10;
export const ALASAN_MAKS = 500;

/** Isian form. Blok JSON disimpan sebagai teks supaya admin bisa menyunting dan melihat galatnya. */
export interface ProviderDraft {
  nama: string;
  base_url: string;
  metode: MetodeProvider;
  tls_trust: TlsTrust;
  log_mode: LogMode;
  config_schema_version: number;
  body_template: string;
  parameters: string;
  respons_skema: string;
  /** Teks kosong = blok tidak dikirim. */
  data_balikan: string;
  alasan: string;
}

export type DraftField = keyof ProviderDraft;

/** Urutan field di layar; fokus pindah ke field salah pertama menurut urutan ini (AC-9). */
export const FIELD_ORDER: readonly DraftField[] = [
  'nama',
  'base_url',
  'metode',
  'tls_trust',
  'log_mode',
  'config_schema_version',
  'body_template',
  'parameters',
  'respons_skema',
  'data_balikan',
  'alasan',
];

export interface FieldIssue {
  /** Kunci pesan di `provider.form.error.*`. */
  kode: string;
  posisi?: Posisi | null;
  /** Nama secret atau parameter yang dirujuk template (bukan nilai). */
  nama?: string | null;
  /** JSON pointer asli dari server, untuk galat di dalam blok JSON. */
  lokasi?: string;
}

export type DraftErrors = Partial<Record<DraftField, FieldIssue>>;

const json = (value: unknown): string => JSON.stringify(value, null, 2);

export function emptyDraft(): ProviderDraft {
  return {
    nama: '',
    base_url: '',
    metode: 'POST',
    tls_trust: 'system',
    log_mode: 'metadata_only',
    config_schema_version: 1,
    body_template: '',
    parameters: '[]',
    respons_skema: '{}',
    data_balikan: '',
    alasan: '',
  };
}

/** Isi form dari provider yang dibaca; `alasan` selalu kosong (tidak disimpan di riwayat, AC-10). */
export function draftFromProvider(provider: Provider): ProviderDraft {
  return {
    nama: provider.nama,
    base_url: provider.base_url,
    metode: provider.metode,
    tls_trust: provider.tls_trust,
    log_mode: provider.log_mode,
    config_schema_version: provider.config_schema_version,
    body_template: provider.body_template,
    parameters: json(provider.parameters),
    respons_skema: json(provider.respons_skema),
    data_balikan: provider.data_balikan === undefined ? '' : json(provider.data_balikan),
    alasan: '',
  };
}

type Parsed = { ok: true; value: unknown } | { ok: false };

function parse(text: string): Parsed {
  try {
    return { ok: true, value: JSON.parse(text) as unknown };
  } catch {
    return { ok: false };
  }
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

export type BuildResult = { ok: true; body: ProviderConfig } | { ok: false; errors: DraftErrors };

/**
 * Validasi sisi klien yang murah lalu susun badan `ProviderConfig`. Server tetap penjaga terakhir
 * (template, rule, data balikan); di sini hanya hal yang bisa dicek tanpa server.
 */
export function toProviderConfig(draft: ProviderDraft, mode: 'create' | 'update'): BuildResult {
  const errors: DraftErrors = {};

  if (!draft.nama.trim()) errors.nama = { kode: 'wajib' };
  if (!draft.base_url.trim()) errors.base_url = { kode: 'wajib' };
  else if (!/^https:\/\/.+/.test(draft.base_url.trim())) errors.base_url = { kode: 'https_wajib' };
  if (!draft.body_template.trim()) errors.body_template = { kode: 'wajib' };

  const parameters = parse(draft.parameters);
  if (!parameters.ok) errors.parameters = { kode: 'json_tidak_sah' };
  else if (!Array.isArray(parameters.value)) errors.parameters = { kode: 'json_bukan_daftar' };

  const respons = parse(draft.respons_skema);
  if (!respons.ok) errors.respons_skema = { kode: 'json_tidak_sah' };
  else if (!isObject(respons.value)) errors.respons_skema = { kode: 'json_bukan_objek' };

  let dataBalikan: Parsed | null = null;
  if (draft.data_balikan.trim()) {
    dataBalikan = parse(draft.data_balikan);
    if (!dataBalikan.ok) errors.data_balikan = { kode: 'json_tidak_sah' };
    else if (!isObject(dataBalikan.value)) errors.data_balikan = { kode: 'json_bukan_objek' };
  } else if (draft.config_schema_version === 2) {
    errors.data_balikan = { kode: 'required_untuk_versi_2' };
  }

  const alasan = draft.alasan.trim();
  if (mode === 'update' && (alasan.length < ALASAN_MIN || alasan.length > ALASAN_MAKS)) {
    errors.alasan = { kode: 'alasan_rentang' };
  }

  if (Object.keys(errors).length > 0) return { ok: false, errors };

  const body: ProviderConfig = {
    nama: draft.nama.trim(),
    base_url: draft.base_url.trim(),
    metode: draft.metode,
    tls_trust: draft.tls_trust,
    log_mode: draft.log_mode,
    config_schema_version: draft.config_schema_version,
    // Template dikirim persis seperti ditulis: spasi dan baris menentukan `posisi` galat (U-9).
    body_template: draft.body_template,
    parameters: (parameters as { value: unknown }).value as ProviderConfig['parameters'],
    respons_skema: (respons as { value: unknown }).value as ProviderConfig['respons_skema'],
  };
  if (dataBalikan?.ok) body.data_balikan = dataBalikan.value as ProviderConfig['data_balikan'];
  if (mode === 'update') body.alasan = alasan;
  return { ok: true, body };
}

/** Field yang punya teks galat di layar; galat untuk pilihan (select) masuk ringkasan `lainnya`. */
const TEXT_FIELDS = new Set<string>([
  'nama',
  'base_url',
  'body_template',
  'parameters',
  'respons_skema',
  'data_balikan',
  'alasan',
]);

/** Kode server yang punya kalimat sendiri; selebihnya memakai kalimat umum dengan kodenya. */
function kodeKlien(field: DraftField, kode: string): string {
  if (field === 'base_url' && kode === 'format') return 'https_wajib';
  if (field === 'alasan' && (kode === 'wajib' || kode === 'rentang')) return 'alasan_rentang';
  return kode;
}

/**
 * Petakan 422 `validasi_gagal` ke galat per field. Segmen pertama JSON pointer menentukan field
 * (`/parameters/0/rule/pola` → `parameters`), sisanya disimpan sebagai `lokasi`. Galat yang tidak
 * menunjuk field teks form (mis. `config_schema_version`) dikembalikan di `lainnya`.
 */
export function serverErrors(
  problem: Problem | null | undefined,
): { fields: DraftErrors; lainnya: FieldIssue[] } | null {
  if (!problem?.errors?.length) return null;
  const fields: DraftErrors = {};
  const lainnya: FieldIssue[] = [];
  for (const e of problem.errors) {
    const segmen = e.field.replace(/^\/+/, '').split('/');
    const field = segmen[0] ?? '';
    const issue: FieldIssue = {
      kode: e.kode,
      posisi: e.posisi ?? null,
      nama: e.nama ?? null,
      lokasi: e.field,
    };
    if (TEXT_FIELDS.has(field)) {
      const key = field as DraftField;
      // Satu pesan per field: yang pertama dari server yang ditampilkan.
      fields[key] ??= { ...issue, kode: kodeKlien(key, e.kode) };
    } else {
      lainnya.push(issue);
    }
  }
  return { fields, lainnya };
}

/** Field salah pertama menurut urutan layar. */
export function firstInvalid(errors: DraftErrors): DraftField | undefined {
  return FIELD_ORDER.find((field) => errors[field] !== undefined);
}
