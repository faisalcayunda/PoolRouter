import type { Endpoint, EndpointConfig, Problem, Schemas } from '../../api/client';

/**
 * Model form endpoint dan kontrak metadata (S04b, S07b, ARC-103). `PUT /endpoints/{id}` adalah
 * penggantian penuh (V-03): daftar `metadata_fields` dan `parameter_literal` dikirim utuh dari draft
 * setiap kali disimpan. Server tetap penjaga terakhir (aturan M-R1, log_mode terhadap provider);
 * di sini hanya pemeriksaan murah yang bisa dilakukan tanpa server.
 */

export type LogMode = Schemas['LogMode'];
export type TipeMetadata = Schemas['TipeMetadata'];
export type KlasifikasiMetadata = Schemas['KlasifikasiMetadata'];
export type MetadataFieldDef = Schemas['MetadataFieldDef'];

export const LOG_MODE: readonly LogMode[] = ['metadata_only', 'full_payload'];
export const TIPE_METADATA: readonly TipeMetadata[] = [
  'string',
  'integer',
  'boolean',
  'date',
  'enum',
];
export const KLASIFIKASI: readonly KlasifikasiMetadata[] = ['Internal', 'Terbatas', 'Rahasia'];

/** Batas kontrak: `EndpointConfig.alasan` 10..500, `nama_field` paling banyak 64 karakter. */
export const ALASAN_MIN = 10;
export const ALASAN_MAKS = 500;
export const NAMA_FIELD_MAKS = 64;
export const NAMA_FIELD_POLA = /^[a-z][a-z0-9_]*$/;

/** Satu baris field metadata di form. `key` stabil supaya hapus/tambah tidak mengacak fokus. */
export interface MetadataRowDraft {
  key: number;
  nama_field: string;
  tipe: TipeMetadata;
  wajib: boolean;
  klasifikasi: KlasifikasiMetadata;
  kirim_ke_data_balikan: boolean;
  deskripsi: string;
  /** Daftar dipisah koma; hanya dikirim untuk tipe `enum`. */
  nilai_diizinkan: string;
  /** Kosong = tidak dikirim. */
  aturan_validasi: string;
}

export interface EndpointDraft {
  provider_id: string;
  downstream_pool_id: string;
  log_mode: LogMode;
  /** Satu `NAMA=nilai` per baris. */
  parameter_literal: string;
  metadata_fields: MetadataRowDraft[];
  alasan: string;
}

export type TopField = keyof EndpointDraft;
export type RowField = Exclude<keyof MetadataRowDraft, 'key'>;

export interface FieldIssue {
  /** Kunci pesan di `endpoint.form.error.*`. */
  kode: string;
  nama?: string | null;
  batas?: number | null;
}

export interface DraftErrors {
  fields: Partial<Record<TopField, FieldIssue>>;
  /** Galat per baris metadata, dikunci dengan `key` baris. */
  rows: Record<number, Partial<Record<RowField, FieldIssue>>>;
}

/** Urutan field di layar; fokus pindah ke field salah pertama menurut urutan ini. */
export const FIELD_ORDER: readonly TopField[] = [
  'provider_id',
  'downstream_pool_id',
  'log_mode',
  'parameter_literal',
  'metadata_fields',
  'alasan',
];
export const ROW_ORDER: readonly RowField[] = [
  'nama_field',
  'tipe',
  'wajib',
  'klasifikasi',
  'kirim_ke_data_balikan',
  'deskripsi',
  'nilai_diizinkan',
  'aturan_validasi',
];

export const emptyErrors = (): DraftErrors => ({ fields: {}, rows: {} });

export function hasErrors(errors: DraftErrors): boolean {
  return Object.keys(errors.fields).length > 0 || Object.keys(errors.rows).length > 0;
}

export function errorCount(errors: DraftErrors): number {
  return (
    Object.keys(errors.fields).length +
    Object.values(errors.rows).reduce((n, row) => n + Object.keys(row).length, 0)
  );
}

let nextKey = 1;

export function emptyRow(): MetadataRowDraft {
  return {
    key: nextKey++,
    nama_field: '',
    tipe: 'string',
    wajib: false,
    klasifikasi: 'Internal',
    kirim_ke_data_balikan: false,
    deskripsi: '',
    nilai_diizinkan: '',
    aturan_validasi: '',
  };
}

export function emptyDraft(): EndpointDraft {
  return {
    provider_id: '',
    downstream_pool_id: '',
    log_mode: 'metadata_only',
    parameter_literal: '',
    metadata_fields: [],
    alasan: '',
  };
}

function rowFromDef(def: MetadataFieldDef): MetadataRowDraft {
  return {
    key: nextKey++,
    nama_field: def.nama_field,
    tipe: def.tipe,
    wajib: def.wajib,
    klasifikasi: def.klasifikasi,
    kirim_ke_data_balikan: def.kirim_ke_data_balikan,
    deskripsi: def.deskripsi,
    nilai_diizinkan: (def.nilai_diizinkan ?? []).join(', '),
    aturan_validasi: def.aturan_validasi ?? '',
  };
}

/** Isi form dari endpoint yang dibaca; `alasan` selalu kosong (alasan tidak dibawa antar versi). */
export function draftFromEndpoint(endpoint: Endpoint): EndpointDraft {
  return {
    provider_id: endpoint.provider_id,
    downstream_pool_id: endpoint.downstream_pool_id,
    log_mode: endpoint.log_mode,
    parameter_literal: Object.entries(endpoint.parameter_literal)
      .map(([nama, nilai]) => `${nama}=${nilai}`)
      .join('\n'),
    metadata_fields: endpoint.metadata_fields.map(rowFromDef),
    alasan: '',
  };
}

/** `NAMA=nilai` per baris; baris kosong dilewati. Nilai boleh memuat `=` (dipotong di yang pertama). */
export function parseParameterLiteral(
  text: string,
): { ok: true; value: Record<string, string> } | { ok: false; kode: string; nama: string } {
  const value: Record<string, string> = {};
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (!line) continue;
    const eq = line.indexOf('=');
    const nama = eq < 0 ? line : line.slice(0, eq).trim();
    if (eq < 0 || !nama || /\s/.test(nama))
      return { ok: false, kode: 'parameter_format', nama: line };
    if (Object.hasOwn(value, nama)) return { ok: false, kode: 'parameter_duplikat', nama };
    value[nama] = line.slice(eq + 1).trim();
  }
  return { ok: true, value };
}

function splitList(text: string): string[] {
  return text
    .split(',')
    .map((v) => v.trim())
    .filter((v) => v !== '');
}

function toDef(row: MetadataRowDraft): MetadataFieldDef {
  const def: MetadataFieldDef = {
    nama_field: row.nama_field.trim(),
    tipe: row.tipe,
    wajib: row.wajib,
    klasifikasi: row.klasifikasi,
    // Field Rahasia tidak pernah dikirim ke callback data balikan; form menguncinya ke false.
    kirim_ke_data_balikan: row.klasifikasi === 'Rahasia' ? false : row.kirim_ke_data_balikan,
    deskripsi: row.deskripsi.trim(),
  };
  if (row.tipe === 'enum') def.nilai_diizinkan = splitList(row.nilai_diizinkan);
  if (row.aturan_validasi.trim()) def.aturan_validasi = row.aturan_validasi.trim();
  return def;
}

export type BuildResult = { ok: true; body: EndpointConfig } | { ok: false; errors: DraftErrors };

/**
 * `providerLogMode` adalah log_mode provider terpilih bila diketahui: endpoint tidak boleh lebih
 * longgar dari provider-nya (`full_payload` pada provider `metadata_only` ditolak server dengan 422;
 * di sini ditangkap lebih awal supaya admin tidak menunggu satu putaran).
 */
export function toEndpointConfig(
  draft: EndpointDraft,
  mode: 'create' | 'update',
  providerLogMode?: LogMode,
): BuildResult {
  const errors = emptyErrors();

  if (!draft.provider_id) errors.fields.provider_id = { kode: 'wajib' };
  if (!draft.downstream_pool_id) errors.fields.downstream_pool_id = { kode: 'wajib' };
  if (providerLogMode === 'metadata_only' && draft.log_mode === 'full_payload') {
    errors.fields.log_mode = { kode: 'log_mode_lebih_longgar' };
  }

  const parameters = parseParameterLiteral(draft.parameter_literal);
  if (!parameters.ok) {
    errors.fields.parameter_literal = { kode: parameters.kode, nama: parameters.nama };
  }

  const seen = new Set<string>();
  for (const row of draft.metadata_fields) {
    const issues: Partial<Record<RowField, FieldIssue>> = {};
    const nama = row.nama_field.trim();
    if (!nama) issues.nama_field = { kode: 'wajib' };
    else if (nama.length > NAMA_FIELD_MAKS || !NAMA_FIELD_POLA.test(nama)) {
      issues.nama_field = { kode: 'nama_field_format', batas: NAMA_FIELD_MAKS };
    } else if (seen.has(nama)) issues.nama_field = { kode: 'nama_field_duplikat', nama };
    seen.add(nama);
    if (row.tipe === 'enum' && splitList(row.nilai_diizinkan).length === 0) {
      issues.nilai_diizinkan = { kode: 'nilai_diizinkan_wajib' };
    }
    if (Object.keys(issues).length > 0) errors.rows[row.key] = issues;
  }

  const alasan = draft.alasan.trim();
  if (mode === 'update' && (alasan.length < ALASAN_MIN || alasan.length > ALASAN_MAKS)) {
    errors.fields.alasan = { kode: 'alasan_rentang' };
  }

  if (hasErrors(errors) || !parameters.ok) return { ok: false, errors };

  const body: EndpointConfig = {
    provider_id: draft.provider_id,
    downstream_pool_id: draft.downstream_pool_id,
    parameter_literal: parameters.value,
    log_mode: draft.log_mode,
    metadata_fields: draft.metadata_fields.map(toDef),
    config_schema_version: 1,
  };
  if (mode === 'update') body.alasan = alasan;
  return { ok: true, body };
}

const TOP_FIELDS = new Set<string>([
  'provider_id',
  'downstream_pool_id',
  'log_mode',
  'parameter_literal',
  'metadata_fields',
  'alasan',
]);
const ROW_FIELDS = new Set<string>(ROW_ORDER);

/**
 * Petakan 422 ke galat per field. Segmen pertama JSON pointer menentukan field; di bawah
 * `/metadata_fields/<n>/<field>` galat jatuh ke baris ke-n. Problem `metadata_wajib_hilang` (M-R1)
 * tanpa daftar `errors` tetap menjadi galat di daftar metadata. Galat yang tidak menunjuk field
 * form dikembalikan di `lainnya`.
 */
export function serverErrors(
  problem: Problem | null | undefined,
  rows: readonly MetadataRowDraft[],
): { errors: DraftErrors; lainnya: FieldIssue[] } | null {
  const errors = emptyErrors();
  const lainnya: FieldIssue[] = [];
  const list = problem?.errors ?? [];
  if (list.length === 0) {
    if (problem?.kode === 'metadata_wajib_hilang') {
      errors.fields.metadata_fields = { kode: 'wajib_hilang' };
      return { errors, lainnya };
    }
    return null;
  }
  for (const e of list) {
    const segmen = e.field.replace(/^\/+/, '').split('/');
    const field = segmen[0] ?? '';
    const issue: FieldIssue = { kode: e.kode, nama: e.nama ?? null, batas: e.batas ?? null };
    if (field === 'metadata_fields' && /^\d+$/.test(segmen[1] ?? '')) {
      const row = rows[Number(segmen[1])];
      const sub = segmen[2] ?? '';
      if (row && ROW_FIELDS.has(sub)) {
        (errors.rows[row.key] ??= {})[sub as RowField] ??= issue;
        continue;
      }
      lainnya.push(issue);
    } else if (TOP_FIELDS.has(field)) {
      errors.fields[field as TopField] ??= issue;
    } else {
      lainnya.push(issue);
    }
  }
  return { errors, lainnya };
}

/** Field atas yang salah pertama menurut urutan layar. */
export function firstInvalid(errors: DraftErrors): TopField | undefined {
  return FIELD_ORDER.find((field) => errors.fields[field] !== undefined);
}
